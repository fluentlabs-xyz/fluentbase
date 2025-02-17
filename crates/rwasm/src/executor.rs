mod opcodes;

use crate::{
    executor::opcodes::run_the_loop,
    module::RwasmModule2,
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
        bytecode::{
            AddressOffset,
            BlockFuel,
            BranchOffset,
            BranchTableTargets,
            DataSegmentIdx,
            ElementSegmentIdx,
            FuncIdx,
            GlobalIdx,
            Instruction,
            LocalDepth,
            SignatureIdx,
            TableIdx,
        },
        code_map::{FuncHeader, InstructionsRef},
        CompiledFunc,
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
    rwasm::{
        instruction::MAX_INSTRUCTION_SIZE_BYTES,
        BinaryFormat,
        BinaryFormatReader,
        N_MAX_RECURSION_DEPTH,
    },
    store::ResourceLimiterRef,
    table::{ElementSegmentEntity, TableEntity},
    TableType,
    Value,
};
use std::sync::Arc;

pub struct RwasmExecutor<E: SyscallHandler<T>, T> {
    pub(crate) store: RwasmContext<T>,
    phantom_data: PhantomData<E>,
    prev_pc: Option<usize>,
}

impl<E: SyscallHandler<T>, T> RwasmExecutor<E, T> {
    pub fn parse(
        rwasm_bytecode: &[u8],
        fuel_limit: Option<u64>,
        context: T,
    ) -> Result<Self, RwasmError> {
        Ok(Self::new(
            Arc::new(RwasmModule2::new(rwasm_bytecode)),
            fuel_limit,
            context,
        ))
    }

    pub fn new(rwasm_module: Arc<RwasmModule2>, fuel_limit: Option<u64>, context: T) -> Self {
        let store = RwasmContext::new(rwasm_module, fuel_limit, context);
        Self {
            store,
            phantom_data: Default::default(),
            prev_pc: None,
        }
    }

    pub fn run(&mut self) -> Result<i32, RwasmError> {
        let opcode_table = make_instruction_table();
        match run_the_loop(self, opcode_table) {
            Ok(_) => Ok(0),
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

    pub(crate) fn parse_next_instr(&self, offset: usize) -> Instruction {
        let slice = &self.store.module.code_section
            [(self.store.program_counter + offset * MAX_INSTRUCTION_SIZE_BYTES)..];
        Instruction::read_from_slice(slice)
            .unwrap_or_else(|_| unreachable!("rwasm: malformed instruction data"))
    }

    pub(crate) fn fetch_drop_keep(&self, offset: usize) -> DropKeep {
        match self.parse_next_instr(offset) {
            Instruction::Return(drop_keep) => drop_keep,
            _ => unreachable!("rwasm: malformed instruction data"),
        }
    }

    pub(crate) fn fetch_table_index(&self, offset: usize) -> TableIdx {
        match self.parse_next_instr(offset) {
            Instruction::TableGet(table_idx) => table_idx,
            _ => unreachable!("rwasm: malformed instruction data"),
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
        self.next_instr(1);
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
        let address = u32::from(address);
        let base_address = offset.into_inner() + address;
        if let Some(tracer) = self.store.tracer.as_mut() {
            tracer.memory_change(
                base_address,
                len,
                &memory[base_address as usize..(base_address + len) as usize],
            );
        }
        self.next_instr(1);
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn execute_unary(&mut self, f: fn(UntypedValue) -> UntypedValue) {
        self.store.sp.eval_top(f);
        self.next_instr(1);
    }

    #[inline(always)]
    pub(crate) fn execute_binary(&mut self, f: fn(UntypedValue, UntypedValue) -> UntypedValue) {
        self.store.sp.eval_top2(f);
        self.next_instr(1);
    }

    #[inline(always)]
    pub(crate) fn try_execute_unary(
        &mut self,
        f: fn(UntypedValue) -> Result<UntypedValue, TrapCode>,
    ) -> Result<(), TrapCode> {
        self.store.sp.try_eval_top(f)?;
        self.next_instr(1);
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn try_execute_binary(
        &mut self,
        f: fn(UntypedValue, UntypedValue) -> Result<UntypedValue, TrapCode>,
    ) -> Result<(), TrapCode> {
        self.store.sp.try_eval_top2(f)?;
        self.next_instr(1);
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn execute_call_internal(
        &mut self,
        is_nested_call: bool,
        skip: usize,
        func_idx: u32,
    ) -> Result<(), RwasmError> {
        self.next_instr(skip);
        self.store.value_stack.sync_stack_ptr(self.store.sp);
        if is_nested_call {
            if self.store.call_stack.len() > N_MAX_RECURSION_DEPTH {
                return Err(RwasmError::TrapCode(TrapCode::StackOverflow));
            }
            self.store.call_stack.push(self.store.program_counter);
        }
        let instr_ref = self
            .store
            .module
            .func_segments
            .get(func_idx as usize)
            .copied()
            .expect("rwasm: unknown internal function");
        let header = FuncHeader::new(InstructionsRef::uninit(), 0, 0);
        self.store.value_stack.prepare_wasm_call(&header)?;
        self.store.sp = self.store.value_stack.stack_ptr();
        self.store.program_counter = (self.store.module.entrypoint_offset + instr_ref) as usize;
        Ok(())
    }

    #[inline(always)]
    pub fn next_instr(&mut self, offset: usize) {
        self.store.program_counter += offset * MAX_INSTRUCTION_SIZE_BYTES;
    }

    #[inline(always)]
    pub fn branch_to<I: Into<BranchOffset>>(&mut self, branch_offset: I) {
        let branch_offset: BranchOffset = branch_offset.into();
        self.store.program_counter = (self.store.program_counter as isize
            + branch_offset.to_i32() as isize * MAX_INSTRUCTION_SIZE_BYTES as isize)
            as usize;
    }

    #[inline(always)]
    pub fn call_to(&mut self, instr_ref: u32) {
        self.store.program_counter = (instr_ref * MAX_INSTRUCTION_SIZE_BYTES as u32) as usize;
    }

    pub fn store(&self) -> &RwasmContext<T> {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut RwasmContext<T> {
        &mut self.store
    }
}

pub type OpCode<E, T> = fn(&mut RwasmExecutor<E, T>) -> Result<(), RwasmError>;

pub type OpCodeTable<E, T> = [OpCode<E, T>; 0x100];

macro_rules! visit_opcodes_table_inner {
    ($code:expr => $handler:ident) => {
        paste::paste! {
            #[inline]
            fn [< opcode_handler_ $code >]<E: SyscallHandler<T>, T>(executor: &mut RwasmExecutor<E, T>) -> Result<(), RwasmError> {
                opcodes::$handler(executor)
            }
        }
    };
    ($code:expr => $handler:ident(UntypedValue)) => {
        paste::paste! {
            #[inline(always)]
            fn [< opcode_handler_ $code >]<E: SyscallHandler<T>, T>(executor: &mut RwasmExecutor<E, T>) -> Result<(), RwasmError> {
                use byteorder::{ByteOrder, LittleEndian};
                // we skip 1 byte for opcode identiifer
                let value = LittleEndian::read_u64(&executor.store.module.code_section[(executor.store.program_counter + 1)..]);
                opcodes::$handler(executor, UntypedValue::from(value))
            }
        }
    };
    ($code:expr => $handler:ident($value:ident)) => {
        paste::paste! {
            #[inline(always)]
            fn [< opcode_handler_ $code >]<E: SyscallHandler<T>, T>(executor: &mut RwasmExecutor<E, T>) -> Result<(), RwasmError> {
                // we skip 1 byte for opcode identiifer
                let mut binary_format_reader = BinaryFormatReader::new(
                    &executor.store.module.code_section[(executor.store.program_counter + 1)..],
                );
                let value = <$value as BinaryFormat>::read_binary(&mut binary_format_reader)
                    .unwrap_or_else(|_| unreachable!("rwasm: malformed instruction data"));
                opcodes::$handler(executor, value)
            }
        }
    };
}

macro_rules! visit_opcodes_table {
    ($($code:expr => $handler:ident $(($value:ident))?),* $(,)?) => {
        $(
            visit_opcodes_table_inner! {
                $code => $handler$(($value))?
            }
        )*

        #[inline]
        pub const fn make_instruction_table<E: SyscallHandler<T>, T>() -> OpCodeTable<E, T> {
            const {
                let mut table: OpCodeTable<E, T> = [opcodes::visit_unknown; 0x100];
                $(
                    table[$code] = paste::paste! { [< opcode_handler_ $code >] };
                )*
                table
            }
        }
    }
}

visit_opcodes_table! {
    0x00 => visit_local_get(LocalDepth),
    0x01 => visit_local_set(LocalDepth),
    0x02 => visit_local_tee(LocalDepth),
    0x03 => visit_br(BranchOffset),
    0x04 => visit_br_if_eqz(BranchOffset),
    0x05 => visit_br_if_nez(BranchOffset),
    0x06 => visit_br_adjust(BranchOffset),
    0x07 => visit_br_adjust_if_nez(BranchOffset),
    0x08 => visit_br_table(BranchTableTargets),
    0x09 => visit_unreachable,
    0x0a => visit_consume_fuel(BlockFuel),
    0x0b => visit_return(DropKeep),
    0x0c => visit_return_if_nez(DropKeep),
    0x0d => visit_return_call_internal(CompiledFunc),
    0x0e => visit_return_call(FuncIdx),
    0x0f => visit_return_call_indirect(SignatureIdx),
    0x10 => visit_call_internal(CompiledFunc),
    0x11 => visit_call(FuncIdx),
    0x12 => visit_call_indirect(SignatureIdx),
    0x13 => visit_signature_check(SignatureIdx),
    0x14 => visit_drop,
    0x15 => visit_select,
    0x16 => visit_global_get(GlobalIdx),
    0x17 => visit_global_set(GlobalIdx),
    0x18 => visit_i32_load(AddressOffset),
    0x19 => visit_i64_load(AddressOffset),
    0x1a => visit_f32_load(AddressOffset),
    0x1b => visit_f64_load(AddressOffset),
    0x1c => visit_i32_load8_s(AddressOffset),
    0x1d => visit_i32_load8_u(AddressOffset),
    0x1e => visit_i32_load16_s(AddressOffset),
    0x1f => visit_i32_load16_u(AddressOffset),
    0x20 => visit_i64_load8_s(AddressOffset),
    0x21 => visit_i64_load8_u(AddressOffset),
    0x22 => visit_i64_load16_s(AddressOffset),
    0x23 => visit_i64_load16_u(AddressOffset),
    0x24 => visit_i64_load32_s(AddressOffset),
    0x25 => visit_i64_load32_u(AddressOffset),
    0x26 => visit_i32_store(AddressOffset),
    0x27 => visit_i64_store(AddressOffset),
    0x28 => visit_f32_store(AddressOffset),
    0x29 => visit_f64_store(AddressOffset),
    0x2a => visit_i32_store8(AddressOffset),
    0x2b => visit_i32_store16(AddressOffset),
    0x2c => visit_i64_store8(AddressOffset),
    0x2d => visit_i64_store16(AddressOffset),
    0x2e => visit_i64_store32(AddressOffset),
    0x2f => visit_memory_size,
    0x30 => visit_memory_grow,
    0x31 => visit_memory_fill,
    0x32 => visit_memory_copy,
    0x33 => visit_memory_init(DataSegmentIdx),
    0x34 => visit_data_drop(DataSegmentIdx),
    0x35 => visit_table_size(TableIdx),
    0x36 => visit_table_grow(TableIdx),
    0x37 => visit_table_fill(TableIdx),
    0x38 => visit_table_get(TableIdx),
    0x39 => visit_table_set(TableIdx),
    0x3a => visit_table_copy(TableIdx),
    0x3b => visit_table_init(ElementSegmentIdx),
    0x3c => visit_elem_drop(ElementSegmentIdx),
    0x3d => visit_ref_func(FuncIdx),
    0x3e => visit_i32_i64_const(UntypedValue),
    0x3f => visit_i32_i64_const(UntypedValue),
    0x40 => visit_i32_i64_const(UntypedValue),
    0x41 => visit_i32_i64_const(UntypedValue),
    0x42 => visit_i32_eqz,
    0x43 => visit_i32_eq,
    0x44 => visit_i32_ne,
    0x45 => visit_i32_lt_s,
    0x46 => visit_i32_lt_u,
    0x47 => visit_i32_gt_s,
    0x48 => visit_i32_gt_u,
    0x49 => visit_i32_le_s,
    0x4a => visit_i32_le_u,
    0x4b => visit_i32_ge_s,
    0x4c => visit_i32_ge_u,
    0x4d => visit_i64_eqz,
    0x4e => visit_i64_eq,
    0x4f => visit_i64_ne,
    0x50 => visit_i64_lt_s,
    0x51 => visit_i64_lt_u,
    0x52 => visit_i64_gt_s,
    0x53 => visit_i64_gt_u,
    0x54 => visit_i64_le_s,
    0x55 => visit_i64_le_u,
    0x56 => visit_i64_ge_s,
    0x57 => visit_i64_ge_u,
    0x58 => visit_f32_eq,
    0x59 => visit_f32_ne,
    0x5a => visit_f32_lt,
    0x5b => visit_f32_gt,
    0x5c => visit_f32_le,
    0x5d => visit_f32_ge,
    0x5e => visit_f64_eq,
    0x5f => visit_f64_ne,
    0x60 => visit_f64_lt,
    0x61 => visit_f64_gt,
    0x62 => visit_f64_le,
    0x63 => visit_f64_ge,
    0x64 => visit_i32_clz,
    0x65 => visit_i32_ctz,
    0x66 => visit_i32_popcnt,
    0x67 => visit_i32_add,
    0x68 => visit_i32_sub,
    0x69 => visit_i32_mul,
    0x6a => visit_i32_div_s,
    0x6b => visit_i32_div_u,
    0x6c => visit_i32_rem_s,
    0x6d => visit_i32_rem_u,
    0x6e => visit_i32_and,
    0x6f => visit_i32_or,
    0x70 => visit_i32_xor,
    0x71 => visit_i32_shl,
    0x72 => visit_i32_shr_s,
    0x73 => visit_i32_shr_u,
    0x74 => visit_i32_rotl,
    0x75 => visit_i32_rotr,
    0x76 => visit_i64_clz,
    0x77 => visit_i64_ctz,
    0x78 => visit_i64_popcnt,
    0x79 => visit_i64_add,
    0x7a => visit_i64_sub,
    0x7b => visit_i64_mul,
    0x7c => visit_i64_div_s,
    0x7d => visit_i64_div_u,
    0x7e => visit_i64_rem_s,
    0x7f => visit_i64_rem_u,
    0x80 => visit_i64_and,
    0x81 => visit_i64_or,
    0x82 => visit_i64_xor,
    0x83 => visit_i64_shl,
    0x84 => visit_i64_shr_s,
    0x85 => visit_i64_shr_u,
    0x86 => visit_i64_rotl,
    0x87 => visit_i64_rotr,
    0x88 => visit_f32_abs,
    0x89 => visit_f32_neg,
    0x8a => visit_f32_ceil,
    0x8b => visit_f32_floor,
    0x8c => visit_f32_trunc,
    0x8d => visit_f32_nearest,
    0x8e => visit_f32_sqrt,
    0x8f => visit_f32_add,
    0x90 => visit_f32_sub,
    0x91 => visit_f32_mul,
    0x92 => visit_f32_div,
    0x93 => visit_f32_min,
    0x94 => visit_f32_max,
    0x95 => visit_f32_copysign,
    0x96 => visit_f64_abs,
    0x97 => visit_f64_neg,
    0x98 => visit_f64_ceil,
    0x99 => visit_f64_floor,
    0x9a => visit_f64_trunc,
    0x9b => visit_f64_nearest,
    0x9c => visit_f64_sqrt,
    0x9d => visit_f64_add,
    0x9e => visit_f64_sub,
    0x9f => visit_f64_mul,
    0xa0 => visit_f64_div,
    0xa1 => visit_f64_min,
    0xa2 => visit_f64_max,
    0xa3 => visit_f64_copysign,
    0xa4 => visit_i32_wrap_i64,
    0xa5 => visit_i32_trunc_f32_s,
    0xa6 => visit_i32_trunc_f32_u,
    0xa7 => visit_i32_trunc_f64_s,
    0xa8 => visit_i32_trunc_f64_u,
    0xa9 => visit_i64_extend_i32_s,
    0xaa => visit_i64_extend_i32_u,
    0xab => visit_i64_trunc_f32_s,
    0xac => visit_i64_trunc_f32_u,
    0xad => visit_i64_trunc_f64_s,
    0xae => visit_i64_trunc_f64_u,
    0xaf => visit_f32_convert_i32_s,
    0xb0 => visit_f32_convert_i32_u,
    0xb1 => visit_f32_convert_i64_s,
    0xb2 => visit_f32_convert_i64_u,
    0xb3 => visit_f32_demote_f64,
    0xb4 => visit_f64_convert_i32_s,
    0xb5 => visit_f64_convert_i32_u,
    0xb6 => visit_f64_convert_i64_s,
    0xb7 => visit_f64_convert_i64_u,
    0xb8 => visit_f64_promote_f32,
    0xb9 => visit_i32_extend8_s,
    0xba => visit_i32_extend16_s,
    0xbb => visit_i64_extend8_s,
    0xbc => visit_i64_extend16_s,
    0xbd => visit_i64_extend32_s,
    0xbe => visit_i32_trunc_sat_f32_s,
    0xbf => visit_i32_trunc_sat_f32_u,
    0xc0 => visit_i32_trunc_sat_f64_s,
    0xc1 => visit_i32_trunc_sat_f64_u,
    0xc2 => visit_i64_trunc_sat_f32_s,
    0xc3 => visit_i64_trunc_sat_f32_u,
    0xc4 => visit_i64_trunc_sat_f64_s,
    0xc5 => visit_i64_trunc_sat_f64_u,
}
