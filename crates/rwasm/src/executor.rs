use crate::{
    always_failing_syscall_handler,
    config::ExecutorConfig,
    instr_ptr::InstructionPtr,
    memory::GlobalMemory,
    module::{InstructionData, RwasmModule2},
    opcodes::run_the_loop,
    types::RwasmError,
    SyscallHandler,
    FUNC_REF_OFFSET,
    N_DEFAULT_STACK_SIZE,
    N_MAX_STACK_SIZE,
    N_MAX_TABLE_SIZE,
    TABLE_ELEMENT_NULL,
};
use core::mem::replace;
use hashbrown::HashMap;
use revm_interpreter::{SharedMemory, EMPTY_SHARED_MEMORY};
use rwasm::{
    core::{TrapCode, UntypedValue, ValueType, N_MAX_MEMORY_PAGES},
    engine::{
        bytecode::{
            AddressOffset,
            DataSegmentIdx,
            ElementSegmentIdx,
            GlobalIdx,
            SignatureIdx,
            TableIdx,
        },
        code_map::{FuncHeader, InstructionsRef},
        stack::{ValueStack, ValueStackPtr},
        DropKeep,
        FuelCosts,
        Tracer,
    },
    memory::DataSegmentEntity,
    module::{
        ConstExpr,
        DataSegment,
        DataSegmentKind,
        ElementSegment,
        ElementSegmentItems,
        ElementSegmentKind,
    },
    rwasm::N_MAX_RECURSION_DEPTH,
    store::ResourceLimiterRef,
    table::{ElementSegmentEntity, TableEntity},
    MemoryType,
    TableType,
    Value,
};
use std::sync::Arc;

pub struct RwasmExecutor<T> {
    // function segments
    pub(crate) module: Arc<RwasmModule2>,
    pub(crate) config: ExecutorConfig,
    // execution context information
    pub(crate) consumed_fuel: u64,
    pub(crate) refunded_fuel: i64,
    pub(crate) value_stack: ValueStack,
    pub(crate) sp: ValueStackPtr,
    pub(crate) global_memory: GlobalMemory,
    pub(crate) ip: InstructionPtr,
    pub(crate) context: T,
    pub(crate) tracer: Option<Tracer>,
    pub(crate) fuel_costs: FuelCosts,
    // rwasm modified segments
    pub(crate) global_variables: HashMap<GlobalIdx, UntypedValue>,
    pub(crate) tables: HashMap<TableIdx, TableEntity>,
    pub(crate) data_segments: HashMap<DataSegmentIdx, DataSegmentEntity>,
    pub(crate) elements: HashMap<ElementSegmentIdx, ElementSegmentEntity>,
    // list of nested calls return pointers
    pub(crate) call_stack: Vec<InstructionPtr>,
    // the last used signature (needed for indirect calls type checks)
    pub(crate) last_signature: Option<SignatureIdx>,
    pub(crate) next_result: Option<Result<i32, RwasmError>>,
    pub(crate) stop_exec: bool,
    pub(crate) syscall_handler: SyscallHandler<T>,
}

impl<T> RwasmExecutor<T> {
    pub fn parse(
        rwasm_bytecode: &[u8],
        shared_memory: SharedMemory,
        config: ExecutorConfig,
        context: T,
    ) -> Result<Self, RwasmError> {
        Ok(Self::new(
            Arc::new(RwasmModule2::new(rwasm_bytecode)),
            shared_memory,
            config,
            context,
        ))
    }

    pub fn new(
        module: Arc<RwasmModule2>,
        shared_memory: SharedMemory,
        config: ExecutorConfig,
        context: T,
    ) -> Self {
        // create stack with sp
        let mut value_stack = ValueStack::new(N_DEFAULT_STACK_SIZE, N_MAX_STACK_SIZE);
        let sp = value_stack.stack_ptr();

        // assign sp to the position inside code section
        let mut ip = InstructionPtr::new(module.code_section.as_ptr(), module.instr_data.as_ptr());
        ip.add(module.source_pc as usize);

        // create global memory
        let mut resource_limiter_ref = ResourceLimiterRef::default();
        let global_memory = GlobalMemory::new(
            shared_memory,
            MemoryType::new(0, Some(N_MAX_MEMORY_PAGES)).expect("rwasm: bad initial memory"),
            &mut resource_limiter_ref,
        )
        .expect("rwasm: bad initial memory");

        // create the main element segment (index 0) from the module elements
        let mut element_segments = HashMap::new();
        element_segments.insert(
            ElementSegmentIdx::from(0u32),
            ElementSegmentEntity::from(&ElementSegment {
                kind: ElementSegmentKind::Passive,
                ty: ValueType::I32,
                items: ElementSegmentItems {
                    exprs: module
                        .element_section
                        .iter()
                        .map(|v| ConstExpr::from_const((*v + FUNC_REF_OFFSET).into()))
                        .collect(),
                },
            }),
        );

        let tracer = if config.trace_enabled {
            Some(Tracer::default())
        } else {
            None
        };

        Self {
            module,
            config,
            consumed_fuel: 0,
            refunded_fuel: 0,
            value_stack,
            sp,
            global_memory,
            ip,
            context,
            tracer,
            fuel_costs: Default::default(),
            global_variables: Default::default(),
            tables: Default::default(),
            data_segments: Default::default(),
            elements: element_segments,
            call_stack: vec![],
            last_signature: None,
            next_result: None,
            stop_exec: false,
            syscall_handler: always_failing_syscall_handler,
        }
    }

    pub fn set_syscall_handler(&mut self, handler: SyscallHandler<T>) {
        self.syscall_handler = handler;
    }

    pub fn program_counter(&self) -> u32 {
        self.ip.pc()
    }

    pub fn reset(&mut self, pc: Option<usize>) {
        let mut ip = InstructionPtr::new(
            self.module.code_section.as_ptr(),
            self.module.instr_data.as_ptr(),
        );
        ip.add(pc.unwrap_or(self.module.source_pc as usize));
        self.ip = ip;
        self.consumed_fuel = 0;
        self.value_stack.drain();
        self.sp = self.value_stack.stack_ptr();
        self.call_stack.clear();
        self.last_signature = None;
    }

    pub fn reset_last_signature(&mut self) {
        self.last_signature = None;
    }

    pub fn try_consume_fuel(&mut self, fuel: u64) -> Result<(), RwasmError> {
        let consumed_fuel = self.consumed_fuel.checked_add(fuel).unwrap_or(u64::MAX);
        if let Some(fuel_limit) = self.config.fuel_limit {
            if consumed_fuel > fuel_limit {
                return Err(RwasmError::TrapCode(TrapCode::OutOfFuel));
            }
        }
        self.consumed_fuel = consumed_fuel;
        Ok(())
    }

    pub fn refund_fuel(&mut self, fuel: i64) {
        self.refunded_fuel += fuel;
    }

    pub fn adjust_fuel_limit(&mut self) -> u64 {
        let consumed_fuel = self.consumed_fuel;
        if let Some(fuel_limit) = self.config.fuel_limit.as_mut() {
            *fuel_limit -= self.consumed_fuel;
        }
        self.consumed_fuel = 0;
        consumed_fuel
    }

    pub fn remaining_fuel(&self) -> Option<u64> {
        Some(self.config.fuel_limit? - self.consumed_fuel)
    }

    pub fn fuel_consumed(&self) -> u64 {
        self.consumed_fuel
    }

    pub fn fuel_refunded(&self) -> i64 {
        self.refunded_fuel
    }

    pub fn tracer(&self) -> Option<&Tracer> {
        self.tracer.as_ref()
    }

    pub fn tracer_mut(&mut self) -> Option<&mut Tracer> {
        self.tracer.as_mut()
    }

    pub fn context(&self) -> &T {
        &self.context
    }

    pub fn context_mut(&mut self) -> &mut T {
        &mut self.context
    }

    pub fn run(&mut self) -> Result<i32, RwasmError> {
        match run_the_loop(self) {
            Ok(exit_code) => Ok(exit_code),
            Err(err) => match err {
                RwasmError::ExecutionHalted(exit_code) => Ok(exit_code),
                _ => Err(err),
            },
        }
    }

    pub(crate) fn resolve_table(&mut self, table_idx: TableIdx) -> &mut TableEntity {
        self.tables
            .get_mut(&table_idx)
            .expect("rwasm: missing table")
    }

    pub(crate) fn resolve_table_or_create(&mut self, table_idx: TableIdx) -> &mut TableEntity {
        self.tables
            .entry(table_idx)
            .or_insert_with(Self::empty_table)
    }

    fn empty_table() -> TableEntity {
        let mut dummy_resource_limiter = ResourceLimiterRef::default();
        TableEntity::new(
            TableType::new(ValueType::I32, 0, Some(N_MAX_TABLE_SIZE as u32)),
            Value::I32(TABLE_ELEMENT_NULL as i32),
            &mut dummy_resource_limiter,
        )
        .unwrap()
    }

    fn empty_element_segment() -> ElementSegmentEntity {
        ElementSegmentEntity::from(&ElementSegment {
            kind: ElementSegmentKind::Passive,
            ty: ValueType::I32,
            items: ElementSegmentItems { exprs: [].into() },
        })
    }

    fn empty_data_segment() -> DataSegmentEntity {
        DataSegmentEntity::from(&DataSegment {
            kind: DataSegmentKind::Passive,
            bytes: [0x1].into(),
        })
    }

    pub(crate) fn resolve_data_or_create(
        &mut self,
        data_segment_idx: DataSegmentIdx,
    ) -> &mut DataSegmentEntity {
        self.data_segments
            .entry(data_segment_idx)
            .or_insert_with(Self::empty_data_segment)
    }

    pub(crate) fn resolve_element_or_create(
        &mut self,
        element_idx: ElementSegmentIdx,
    ) -> &mut ElementSegmentEntity {
        self.elements
            .entry(element_idx)
            .or_insert_with(Self::empty_element_segment)
    }

    pub(crate) fn resolve_table_with_element_or_create(
        &mut self,
        table_idx: TableIdx,
        element_idx: ElementSegmentIdx,
    ) -> (&mut TableEntity, &mut ElementSegmentEntity) {
        let table_entity = self
            .tables
            .entry(table_idx)
            .or_insert_with(Self::empty_table);
        let element_entity = self
            .elements
            .entry(element_idx)
            .or_insert_with(Self::empty_element_segment);
        (table_entity, element_entity)
    }

    pub(crate) fn fetch_drop_keep(&self, offset: usize) -> DropKeep {
        let mut addr: InstructionPtr = self.ip;
        addr.add(offset);
        match addr.data() {
            InstructionData::DropKeep(drop_keep) => *drop_keep,
            _ => unreachable!("rwasm: can't extract drop keep"),
        }
    }

    pub(crate) fn fetch_table_index(&self, offset: usize) -> TableIdx {
        let mut addr: InstructionPtr = self.ip;
        addr.add(offset);
        match addr.data() {
            InstructionData::TableIdx(table_idx) => *table_idx,
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
    pub(crate) fn try_execute_unary(
        &mut self,
        f: fn(UntypedValue) -> Result<UntypedValue, TrapCode>,
    ) -> Result<(), TrapCode> {
        self.sp.try_eval_top(f)?;
        self.ip.add(1);
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn try_execute_binary(
        &mut self,
        f: fn(UntypedValue, UntypedValue) -> Result<UntypedValue, TrapCode>,
    ) -> Result<(), TrapCode> {
        self.sp.try_eval_top2(f)?;
        self.ip.add(1);
        Ok(())
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
            .module
            .func_segments
            .get(func_idx as usize)
            .copied()
            .expect("rwasm: unknown internal function");
        let header = FuncHeader::new(InstructionsRef::uninit(), 0, 0);
        self.value_stack.prepare_wasm_call(&header)?;
        self.sp = self.value_stack.stack_ptr();
        self.ip = InstructionPtr::new(
            self.module.code_section.as_ptr(),
            self.module.instr_data.as_ptr(),
        );
        self.ip.add(instr_ref as usize);
        Ok(())
    }

    pub fn take_shared_memory(&mut self) -> SharedMemory {
        replace(&mut self.global_memory.shared_memory, EMPTY_SHARED_MEMORY)
    }

    pub fn insert_shared_memory(&mut self, shared_memory: SharedMemory) {
        self.global_memory.shared_memory = shared_memory;
    }
}
