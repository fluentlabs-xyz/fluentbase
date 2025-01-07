use crate::executor::vec::Vec;
use alloc::vec;
use hashbrown::HashMap;
use rwasm::{
    core::{TrapCode, UntypedValue, ValueType},
    engine::{
        bytecode::{
            AddressOffset,
            DataSegmentIdx,
            ElementSegmentIdx,
            GlobalIdx,
            Instruction,
            SignatureIdx,
            TableIdx,
        },
        code_map::{FuncHeader, InstructionPtr, InstructionsRef},
        stack::{ValueStack, ValueStackPtr},
        DropKeep,
        Tracer,
    },
    errors::MemoryError,
    memory::{DataSegmentEntity, MemoryEntity},
    module::{ConstExpr, ElementSegment, ElementSegmentItems, ElementSegmentKind},
    rwasm::{BinaryFormatError, RwasmModule, N_MAX_RECURSION_DEPTH},
    store::ResourceLimiterRef,
    table::{ElementSegmentEntity, TableEntity},
    MemoryType,
    TableType,
    Value,
};

#[derive(Debug)]
pub enum RwasmError {
    BinaryFormatError(BinaryFormatError),
    TrapCode(TrapCode),
    UnknownExternalFunction(u32),
    ExecutionHalted(i32),
    MemoryError(MemoryError),
    InputOutOfBounds,
}

impl From<BinaryFormatError> for RwasmError {
    fn from(value: BinaryFormatError) -> Self {
        Self::BinaryFormatError(value)
    }
}
impl From<TrapCode> for RwasmError {
    fn from(value: TrapCode) -> Self {
        Self::TrapCode(value)
    }
}
impl From<MemoryError> for RwasmError {
    fn from(value: MemoryError) -> Self {
        Self::MemoryError(value)
    }
}

pub trait SyscallHandler {
    fn call_function(
        &mut self,
        func_idx: u32,
        sp: &mut ValueStackPtr,
        global_memory: &mut MemoryEntity,
    ) -> Result<(), RwasmError>;
}

#[derive(Default)]
pub struct AlwaysFailingSyscallHandler;

impl SyscallHandler for AlwaysFailingSyscallHandler {
    fn call_function(
        &mut self,
        func_idx: u32,
        _sp: &mut ValueStackPtr,
        _global_memory: &mut MemoryEntity,
    ) -> Result<(), RwasmError> {
        Err(RwasmError::UnknownExternalFunction(func_idx))
    }
}

pub struct RwasmExecutor<'a, E: SyscallHandler> {
    pub(crate) rwasm_module: RwasmModule,
    pub(crate) syscall_handler: Option<&'a mut E>,
    pub(crate) func_segments: Vec<u32>,
    pub(crate) sp: ValueStackPtr,
    pub(crate) value_stack: ValueStack,
    pub(crate) ip: InstructionPtr,
    pub(crate) global_memory: MemoryEntity,
    pub(crate) global_variables: HashMap<GlobalIdx, UntypedValue>,
    pub(crate) tables: HashMap<TableIdx, TableEntity>,
    pub(crate) data_segments: HashMap<DataSegmentIdx, DataSegmentEntity>,
    pub(crate) element_segments: HashMap<ElementSegmentIdx, ElementSegmentEntity>,
    pub(crate) call_stack: Vec<InstructionPtr>,
    pub(crate) last_signature: Option<SignatureIdx>,
    pub(crate) tracer: Option<&'a mut Tracer>,
    pub(crate) fuel_limit: Option<u64>,
    pub(crate) fuel_consumed: u64,
}

impl<'a, E: SyscallHandler> RwasmExecutor<'a, E> {
    pub fn parse(
        rwasm_bytecode: &[u8],
        syscall_handler: Option<&'a mut E>,
        fuel_limit: Option<u64>,
    ) -> Result<Self, RwasmError> {
        let mut rwasm_module = RwasmModule::new(rwasm_bytecode)?;
        // a special case for an empty code section, just don't execute
        if rwasm_module.code_section.instr.is_empty() {
            rwasm_module.code_section.op_return();
        }
        Ok(Self::new(rwasm_module, syscall_handler, fuel_limit))
    }

    pub fn new(
        rwasm_module: RwasmModule,
        syscall_handler: Option<&'a mut E>,
        fuel_limit: Option<u64>,
    ) -> Self {
        let mut func_segments = vec![0u32];
        let mut total_func_len = 0u32;
        for func_len in rwasm_module
            .func_section
            .iter()
            .take(rwasm_module.func_section.len() - 1)
        {
            total_func_len += *func_len;
            func_segments.push(total_func_len);
        }
        let source_pc = func_segments
            .last()
            .copied()
            .expect("rwasm: empty function section");

        let mut value_stack = ValueStack::default();

        let mut resource_limiter_ref = ResourceLimiterRef::default();
        let global_memory = MemoryEntity::new(
            MemoryType::new(0, Some(1024)).expect("rwasm: bad initial memory"),
            &mut resource_limiter_ref,
        )
        .expect("rwasm: bad initial memory");

        let sp = value_stack.stack_ptr();
        let mut ip = InstructionPtr::new(
            rwasm_module.code_section.instr.as_ptr(),
            rwasm_module.code_section.metas.as_ptr(),
        );
        ip.add(source_pc as usize);

        let last_signature: Option<SignatureIdx> = None;

        // create the main element segment (index 0) from the module elements
        let mut element_segments = HashMap::new();
        element_segments.insert(
            ElementSegmentIdx::from(0u32),
            ElementSegmentEntity::from(&ElementSegment {
                kind: ElementSegmentKind::Passive,
                ty: ValueType::I32,
                items: ElementSegmentItems {
                    exprs: rwasm_module
                        .element_section
                        .iter()
                        .map(|v| ConstExpr::from_const((*v).into()))
                        .collect(),
                },
            }),
        );

        Self {
            rwasm_module,
            syscall_handler,
            func_segments,
            sp,
            value_stack,
            ip,
            global_memory,
            global_variables: HashMap::new(),
            tables: HashMap::new(),
            data_segments: HashMap::new(),
            element_segments,
            call_stack: Vec::new(),
            last_signature,
            tracer: None,
            fuel_limit,
            fuel_consumed: 0,
        }
    }

    pub fn run(&mut self) -> Result<i32, RwasmError> {
        match self.run_the_loop() {
            Ok(exit_code) => Ok(exit_code),
            Err(err) => match err {
                RwasmError::ExecutionHalted(exit_code) => Ok(exit_code),
                _ => Err(err),
            },
        }
    }

    pub(crate) fn resolve_table_or_create(&mut self, table_idx: TableIdx) -> &mut TableEntity {
        self.tables.entry(table_idx).or_insert_with(|| {
            let mut dummy_resource_limiter = ResourceLimiterRef::default();
            TableEntity::new(
                TableType::new(ValueType::I32, 0, None),
                Value::I32(0),
                &mut dummy_resource_limiter,
            )
            .unwrap()
        })
    }

    pub(crate) fn resolve_element_or_create(
        &mut self,
        element_idx: ElementSegmentIdx,
    ) -> &mut ElementSegmentEntity {
        self.element_segments.entry(element_idx).or_insert_with(|| {
            ElementSegmentEntity::from(&ElementSegment {
                kind: ElementSegmentKind::Passive,
                ty: ValueType::I32,
                items: ElementSegmentItems { exprs: [].into() },
            })
        })
    }

    pub(crate) fn resolve_table_with_element_or_create(
        &mut self,
        table_idx: TableIdx,
        element_idx: ElementSegmentIdx,
    ) -> (&mut TableEntity, &mut ElementSegmentEntity) {
        let table_entity = self.tables.entry(table_idx).or_insert_with(|| {
            let mut dummy_resource_limiter = ResourceLimiterRef::default();
            TableEntity::new(
                TableType::new(ValueType::I32, 0, None),
                Value::I32(0),
                &mut dummy_resource_limiter,
            )
            .unwrap()
        });
        let element_entity = self.element_segments.entry(element_idx).or_insert_with(|| {
            ElementSegmentEntity::from(&ElementSegment {
                kind: ElementSegmentKind::Passive,
                ty: ValueType::I32,
                items: ElementSegmentItems { exprs: [].into() },
            })
        });
        (table_entity, element_entity)
    }

    pub(crate) fn fetch_drop_keep(&self, offset: usize) -> DropKeep {
        let mut addr: InstructionPtr = self.ip;
        addr.add(offset);
        match addr.get() {
            Instruction::Return(drop_keep) => *drop_keep,
            _ => unreachable!("rwasm: can't extract drop keep"),
        }
    }

    pub(crate) fn fetch_table_index(&self, offset: usize) -> TableIdx {
        let mut addr: InstructionPtr = self.ip;
        addr.add(offset);
        match addr.get() {
            Instruction::TableGet(table_idx) => *table_idx,
            _ => unreachable!("rwasm: can't extract table index"),
        }
    }

    #[inline(always)]
    pub(crate) fn execute_load_extend(
        &mut self,
        offset: AddressOffset,
        load_extend: fn(
            memory: &[u8],
            address: UntypedValue,
            offset: u32,
        ) -> Result<UntypedValue, TrapCode>,
    ) -> Result<(), TrapCode> {
        self.sp.try_eval_top(|address| {
            let memory = self.global_memory.data();
            let value = load_extend(memory, address, offset.into_inner())?;
            Ok(value)
        })?;
        self.ip.add(1);
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn execute_store_wrap(
        &mut self,
        offset: AddressOffset,
        store_wrap: fn(
            memory: &mut [u8],
            address: UntypedValue,
            offset: u32,
            value: UntypedValue,
        ) -> Result<(), TrapCode>,
        len: u32,
    ) -> Result<(), TrapCode> {
        let (address, value) = self.sp.pop2();
        let memory = self.global_memory.data_mut();
        store_wrap(memory, address, offset.into_inner(), value)?;
        self.ip.offset(0);
        let address = u32::from(address);
        let base_address = offset.into_inner() + address;
        if let Some(tracer) = self.tracer.as_mut() {
            tracer.memory_change(
                base_address,
                len,
                &memory[base_address as usize..(base_address + len) as usize],
            );
        }
        self.ip.add(1);
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn execute_unary(&mut self, f: fn(UntypedValue) -> UntypedValue) {
        self.sp.eval_top(f);
        self.ip.add(1);
    }

    #[inline(always)]
    pub(crate) fn execute_binary(&mut self, f: fn(UntypedValue, UntypedValue) -> UntypedValue) {
        self.sp.eval_top2(f);
        self.ip.add(1);
    }

    #[inline(always)]
    pub(crate) fn execute_call_internal(
        &mut self,
        is_nested_call: bool,
        skip: usize,
        func_idx: u32,
    ) -> Result<(), RwasmError> {
        self.ip.add(skip);
        self.value_stack.sync_stack_ptr(self.sp);
        if is_nested_call {
            if self.call_stack.len() > N_MAX_RECURSION_DEPTH {
                return Err(RwasmError::TrapCode(TrapCode::StackOverflow));
            }
            self.call_stack.push(self.ip);
        }
        let instr_ref = self
            .func_segments
            .get(func_idx as usize)
            .copied()
            .expect("rwasm: unknown internal function");
        let header = FuncHeader::new(InstructionsRef::uninit(), 0, 0);
        self.value_stack.prepare_wasm_call(&header)?;
        self.sp = self.value_stack.stack_ptr();
        self.ip = InstructionPtr::new(
            self.rwasm_module.code_section.instr.as_ptr(),
            self.rwasm_module.code_section.metas.as_ptr(),
        );
        self.ip.add(instr_ref as usize);
        Ok(())
    }
}

#[deprecated(since = "0.1.0", note = "use `RwasmExecutor::new` instead")]
pub fn execute_rwasm_module<'a, E: SyscallHandler>(
    rwasm_module: RwasmModule,
    syscall_handler: Option<&'a mut E>,
    fuel_limit: Option<u64>,
) -> Result<i32, RwasmError> {
    RwasmExecutor::<'a, E>::new(rwasm_module, syscall_handler, fuel_limit).run()
}
