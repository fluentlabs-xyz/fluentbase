use crate::{
    config::ExecutorConfig,
    types::RwasmError,
    RwasmContext,
    SyscallHandler,
    N_MAX_TABLE_SIZE,
    TABLE_ELEMENT_NULL,
};
use core::marker::PhantomData;
use rwasm::{
    core::{TrapCode, UntypedValue, ValueType},
    engine::{
        bytecode::{AddressOffset, DataSegmentIdx, ElementSegmentIdx, Instruction, TableIdx},
        code_map::{FuncHeader, InstructionPtr, InstructionsRef},
        DropKeep,
    },
    memory::DataSegmentEntity,
    module::{
        DataSegment,
        DataSegmentKind,
        ElementSegment,
        ElementSegmentItems,
        ElementSegmentKind,
    },
    rwasm::{RwasmModule, RwasmModuleInstance, N_MAX_RECURSION_DEPTH},
    store::ResourceLimiterRef,
    table::{ElementSegmentEntity, TableEntity},
    TableType,
    Value,
};

pub struct RwasmExecutor<E: SyscallHandler<T>, T> {
    pub(crate) store: RwasmContext<T>,
    phantom_data: PhantomData<E>,
}

impl<E: SyscallHandler<T>, T> RwasmExecutor<E, T> {
    pub fn parse(
        rwasm_bytecode: &[u8],
        config: ExecutorConfig,
        context: T,
    ) -> Result<Self, RwasmError> {
        Ok(Self::new(
            RwasmModule::new_or_empty(rwasm_bytecode)?.instantiate(),
            config,
            context,
        ))
    }

    pub fn new(rwasm_module: RwasmModuleInstance, config: ExecutorConfig, context: T) -> Self {
        let store = RwasmContext::new(rwasm_module, config, context);
        Self {
            store,
            phantom_data: Default::default(),
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

    pub(crate) fn resolve_table(&mut self, table_idx: TableIdx) -> &mut TableEntity {
        self.store
            .tables
            .get_mut(&table_idx)
            .expect("rwasm: missing table")
    }

    pub(crate) fn resolve_table_or_create(&mut self, table_idx: TableIdx) -> &mut TableEntity {
        self.store
            .tables
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
        self.store
            .data_segments
            .entry(data_segment_idx)
            .or_insert_with(Self::empty_data_segment)
    }

    pub(crate) fn resolve_element_or_create(
        &mut self,
        element_idx: ElementSegmentIdx,
    ) -> &mut ElementSegmentEntity {
        self.store
            .elements
            .entry(element_idx)
            .or_insert_with(Self::empty_element_segment)
    }

    pub(crate) fn resolve_table_with_element_or_create(
        &mut self,
        table_idx: TableIdx,
        element_idx: ElementSegmentIdx,
    ) -> (&mut TableEntity, &mut ElementSegmentEntity) {
        let table_entity = self
            .store
            .tables
            .entry(table_idx)
            .or_insert_with(Self::empty_table);
        let element_entity = self
            .store
            .elements
            .entry(element_idx)
            .or_insert_with(Self::empty_element_segment);
        (table_entity, element_entity)
    }

    pub(crate) fn fetch_drop_keep(&self, offset: usize) -> DropKeep {
        let mut addr: InstructionPtr = self.store.ip;
        addr.add(offset);
        match addr.get() {
            Instruction::Return(drop_keep) => *drop_keep,
            _ => unreachable!("rwasm: can't extract drop keep"),
        }
    }

    pub(crate) fn fetch_table_index(&self, offset: usize) -> TableIdx {
        let mut addr: InstructionPtr = self.store.ip;
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
        self.store.sp.try_eval_top(|address| {
            let memory = self.store.global_memory.data();
            let value = load_extend(memory, address, offset.into_inner())?;
            Ok(value)
        })?;
        self.store.ip.add(1);
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
        let (address, value) = self.store.sp.pop2();
        let memory = self.store.global_memory.data_mut();
        store_wrap(memory, address, offset.into_inner(), value)?;
        self.store.ip.offset(0);
        let address = u32::from(address);
        let base_address = offset.into_inner() + address;
        if let Some(tracer) = self.store.tracer.as_mut() {
            tracer.memory_change(
                base_address,
                len,
                &memory[base_address as usize..(base_address + len) as usize],
            );
        }
        self.store.ip.add(1);
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn execute_unary(&mut self, f: fn(UntypedValue) -> UntypedValue) {
        self.store.sp.eval_top(f);
        self.store.ip.add(1);
    }

    #[inline(always)]
    pub(crate) fn execute_binary(&mut self, f: fn(UntypedValue, UntypedValue) -> UntypedValue) {
        self.store.sp.eval_top2(f);
        self.store.ip.add(1);
    }

    #[inline(always)]
    pub(crate) fn try_execute_unary(
        &mut self,
        f: fn(UntypedValue) -> Result<UntypedValue, TrapCode>,
    ) -> Result<(), TrapCode> {
        self.store.sp.try_eval_top(f)?;
        self.store.ip.add(1);
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn try_execute_binary(
        &mut self,
        f: fn(UntypedValue, UntypedValue) -> Result<UntypedValue, TrapCode>,
    ) -> Result<(), TrapCode> {
        self.store.sp.try_eval_top2(f)?;
        self.store.ip.add(1);
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn execute_call_internal(
        &mut self,
        is_nested_call: bool,
        skip: usize,
        func_idx: u32,
    ) -> Result<(), RwasmError> {
        self.store.ip.add(skip);
        self.store.value_stack.sync_stack_ptr(self.store.sp);
        if is_nested_call {
            if self.store.call_stack.len() > N_MAX_RECURSION_DEPTH {
                return Err(RwasmError::TrapCode(TrapCode::StackOverflow));
            }
            self.store.call_stack.push(self.store.ip);
        }
        let instr_ref = self
            .store
            .instance
            .func_segments
            .get(func_idx as usize)
            .copied()
            .expect("rwasm: unknown internal function");
        let header = FuncHeader::new(InstructionsRef::uninit(), 0, 0);
        self.store.value_stack.prepare_wasm_call(&header)?;
        self.store.sp = self.store.value_stack.stack_ptr();
        self.store.ip = InstructionPtr::new(
            self.store.instance.module.code_section.instr.as_ptr(),
            self.store.instance.module.code_section.metas.as_ptr(),
        );
        self.store.ip.add(instr_ref as usize);
        Ok(())
    }

    pub fn store(&self) -> &RwasmContext<T> {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut RwasmContext<T> {
        &mut self.store
    }
}
