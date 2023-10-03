use crate::{
    common::UntypedValue,
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
            InstrMeta,
            Instruction,
            LocalDepth,
            TableIdx,
        },
        CompiledFunc,
        ConstRef,
        DropKeep,
    },
    rwasm::{BinaryFormat, BinaryFormatWriter, N_BYTES_PER_MEMORY_PAGE, N_MAX_MEMORY_PAGES},
};
use alloc::{slice::SliceIndex, vec::Vec};
use byteorder::{ByteOrder, LittleEndian};

#[derive(Default, Debug, Clone, PartialEq)]
pub struct InstructionSet {
    pub instr: Vec<Instruction>,
    pub metas: Option<Vec<InstrMeta>>,
    // translate state
    total_locals: Vec<usize>,
    init_memory_size: u32,
    init_memory_pages: u32,
}

impl Into<Vec<u8>> for InstructionSet {
    fn into(mut self) -> Vec<u8> {
        self.finalize(true);
        let mut buffer = vec![0; 65536 * 2];
        let mut binary_writer = BinaryFormatWriter::new(buffer.as_mut_slice());
        let n = self.write_binary(&mut binary_writer).unwrap();
        assert_ne!(n, buffer.len());
        buffer.resize(n, 0);
        buffer
    }
}

macro_rules! impl_opcode {
    ($name:ident, $opcode:ident, $default:expr) => {
        pub fn $name(&mut self) {
            self.push(Instruction::$opcode($default));
        }
    };
    ($name:ident, $opcode:ident($into:ident)) => {
        pub fn $name<I: Into<$into>>(&mut self, value: I) {
            self.push(Instruction::$opcode(value.into()));
        }
    };
    ($name:ident, $opcode:ident($into:ident, $into2:ident)) => {
        pub fn $name<I: Into<$into>, J: Into<$into2>>(&mut self, value: I, value2: J) {
            self.push(Instruction::$opcode(value.into(), value2.into()));
        }
    };
    ($name:ident, $opcode:ident) => {
        pub fn $name(&mut self) {
            self.push(Instruction::$opcode);
        }
    };
}

impl From<Vec<Instruction>> for InstructionSet {
    fn from(value: Vec<Instruction>) -> Self {
        Self {
            instr: value,
            metas: None,
            total_locals: vec![],
            init_memory_size: 0,
            init_memory_pages: 0,
        }
    }
}

impl InstructionSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, opcode: Instruction) -> u32 {
        let opcode_pos = self.len();
        self.instr.push(opcode);
        opcode_pos
    }

    pub fn push_with_meta(&mut self, opcode: Instruction, meta: InstrMeta) -> u32 {
        let opcode_pos = self.push(opcode);
        let metas_len = if let Some(metas) = &mut self.metas {
            metas.push(meta);
            metas.len()
        } else {
            self.metas = Some(vec![meta]);
            1
        };
        assert_eq!(self.instr.len(), metas_len, "instr len and meta mismatched");
        opcode_pos
    }

    pub fn add_memory_pages(&mut self, initial_pages: u32) {
        assert_eq!(self.init_memory_pages, 0);
        self.op_i32_const(initial_pages);
        self.op_memory_grow();
        self.op_drop();
        self.init_memory_pages = initial_pages;
        // we set here 0 because this memory is not used yet
        self.init_memory_size = 0;
    }

    pub fn add_memory(&mut self, mut offset: u32, mut bytes: &[u8]) -> bool {
        // make sure we have enough allocated memory
        let new_size = self.init_memory_size + offset + bytes.len() as u32;
        let total_pages = (new_size + N_BYTES_PER_MEMORY_PAGE - 1) / N_BYTES_PER_MEMORY_PAGE;
        if total_pages > N_MAX_MEMORY_PAGES {
            return false;
        } else if total_pages > self.init_memory_pages {
            self.op_i32_const(total_pages - self.init_memory_pages);
            self.op_memory_grow();
            self.op_drop();
        }
        self.init_memory_size += bytes.len() as u32;
        self.init_memory_pages = total_pages;
        // translate input bytes
        [8, 4, 2, 1].iter().copied().for_each(|chunk_size| {
            let mut it = bytes.chunks_exact(chunk_size);
            while let Some(chunk) = it.next() {
                let value = match chunk_size {
                    8 => LittleEndian::read_u64(chunk),
                    4 => LittleEndian::read_u32(chunk) as u64,
                    2 => LittleEndian::read_u16(chunk) as u64,
                    1 => chunk[0] as u64,
                    _ => unreachable!("not supported chunk size: {}", chunk_size),
                };
                self.op_i32_const(offset);
                self.op_i64_const(value);
                match chunk_size {
                    8 => self.op_i64_store(0u32),
                    4 => self.op_i32_store(0u32),
                    2 => self.op_i32_store16(0u32),
                    1 => self.op_i64_store8(0u32),
                    _ => unreachable!("not supported chunk size: {}", chunk_size),
                }
                offset += chunk_size as u32;
            }
            bytes = it.remainder();
        });
        return true;
    }

    pub fn propagate_locals(&mut self, n: usize) {
        (0..n).for_each(|_| self.op_i32_const(0));
        self.total_locals.push(n);
    }

    pub fn drop_locals(&mut self) {
        let n = self
            .total_locals
            .pop()
            .unwrap_or_else(|| unreachable!("there is no locals on the stack"));
        (0..n).for_each(|_| self.op_drop());
    }

    fn is_return_last(&self) -> bool {
        self.instr
            .last()
            .map(|instr| match instr {
                Instruction::Return(_) => true,
                _ => false,
            })
            .unwrap_or_default()
    }

    pub fn finalize(&mut self, inject_return: bool) {
        // 0 means there is no locals, 1 means main locals, 1+ means error
        if self.total_locals.len() > 1 {
            unreachable!("missing [drop_locals] call/s somewhere");
        } else if self.total_locals.len() == 1 {
            self.drop_locals();
        }
        // inject return as a last opcode before unreachable
        if inject_return && !self.is_return_last() {
            self.op_return();
        }
        // inject unreachable in the end of file to be sure that return is presented
        self.op_unreachable();
    }

    pub fn has_meta(&self) -> bool {
        self.metas.is_some()
    }

    pub fn get<I>(&self, index: I) -> Option<&Instruction>
    where
        I: SliceIndex<[Instruction], Output = Instruction>,
    {
        self.instr.get(index)
    }

    pub fn get_mut<I>(&mut self, index: I) -> Option<&mut Instruction>
    where
        I: SliceIndex<[Instruction], Output = Instruction>,
    {
        self.instr.get_mut(index)
    }

    pub fn count_globals(&self) -> u32 {
        self.instr
            .iter()
            .filter_map(|opcode| match opcode {
                Instruction::GlobalGet(index) | Instruction::GlobalSet(index) => {
                    Some(index.to_u32())
                }
                _ => None,
            })
            .max()
            .map(|v| v + 1)
            .unwrap_or_default()
    }

    pub fn count_tables(&self) -> u32 {
        self.instr
            .iter()
            .filter_map(|opcode| match opcode {
                Instruction::TableSize(index)
                | Instruction::TableGrow(index)
                | Instruction::TableFill(index)
                | Instruction::TableGet(index)
                | Instruction::TableSet(index)
                | Instruction::TableCopy(index) => Some(index.to_u32()),
                _ => None,
            })
            .max()
            .map(|v| v + 1)
            .unwrap_or_default()
    }

    pub fn instr(&self) -> &Vec<Instruction> {
        &self.instr
    }

    pub fn instr_mut(&mut self) -> &mut Vec<Instruction> {
        &mut self.instr
    }

    pub fn len(&self) -> u32 {
        self.instr.len() as u32
    }

    impl_opcode!(op_local_get, LocalGet(LocalDepth));
    impl_opcode!(op_local_set, LocalSet(LocalDepth));
    impl_opcode!(op_local_tee, LocalTee(LocalDepth));
    impl_opcode!(op_br, Br(BranchOffset));
    impl_opcode!(op_br_if_eqz, BrIfEqz(BranchOffset));
    impl_opcode!(op_br_if_nez, BrIfNez(BranchOffset));
    impl_opcode!(op_br_adjust, BrAdjust(BranchOffset));
    impl_opcode!(op_br_adjust_if_nez, BrAdjustIfNez(BranchOffset));
    impl_opcode!(op_br_table, BrTable(BranchTableTargets));
    impl_opcode!(op_unreachable, Unreachable);
    impl_opcode!(op_consume_fuel, ConsumeFuel(BlockFuel));
    impl_opcode!(op_return, Return, DropKeep::none());
    impl_opcode!(op_br_indirect, BrIndirect);

    impl_opcode!(op_return_if_nez, ReturnIfNez, DropKeep::none());
    impl_opcode!(op_return_call_internal, ReturnCallInternal(CompiledFunc));
    impl_opcode!(op_return_call, ReturnCall(FuncIdx));
    impl_opcode!(op_return_call_indirect, ReturnCallIndirectUnsafe(TableIdx));
    impl_opcode!(op_call_internal, CallInternal(CompiledFunc));
    impl_opcode!(op_call, Call(FuncIdx));
    impl_opcode!(op_call_indirect, CallIndirectUnsafe(TableIdx));
    impl_opcode!(op_drop, Drop);
    impl_opcode!(op_select, Select);
    impl_opcode!(op_global_get, GlobalGet(GlobalIdx));
    impl_opcode!(op_global_set, GlobalSet(GlobalIdx));
    impl_opcode!(op_i32_load, I32Load(AddressOffset));
    impl_opcode!(op_i64_load, I64Load(AddressOffset));
    impl_opcode!(op_f32_load, F32Load(AddressOffset));
    impl_opcode!(op_f64_load, F64Load(AddressOffset));
    impl_opcode!(op_i32_load8_s, I32Load8S(AddressOffset));
    impl_opcode!(op_i32_load8_u, I32Load8U(AddressOffset));
    impl_opcode!(op_i32_load16_s, I32Load16S(AddressOffset));
    impl_opcode!(op_i32_load16_u, I32Load16U(AddressOffset));
    impl_opcode!(op_i64_load8_s, I64Load8S(AddressOffset));
    impl_opcode!(op_i64_load8_u, I64Load8U(AddressOffset));
    impl_opcode!(op_i64_load16_s, I64Load16S(AddressOffset));
    impl_opcode!(op_i64_load16_u, I64Load16U(AddressOffset));
    impl_opcode!(op_i64_load32_s, I64Load32S(AddressOffset));
    impl_opcode!(op_i64_load32_u, I64Load32U(AddressOffset));
    impl_opcode!(op_i32_store, I32Store(AddressOffset));
    impl_opcode!(op_i64_store, I64Store(AddressOffset));
    impl_opcode!(op_f32_store, F32Store(AddressOffset));
    impl_opcode!(op_f64_store, F64Store(AddressOffset));
    impl_opcode!(op_i32_store8, I32Store8(AddressOffset));
    impl_opcode!(op_i32_store16, I32Store16(AddressOffset));
    impl_opcode!(op_i64_store8, I64Store8(AddressOffset));
    impl_opcode!(op_i64_store16, I64Store16(AddressOffset));
    impl_opcode!(op_i64_store32, I64Store32(AddressOffset));
    impl_opcode!(op_memory_size, MemorySize);
    impl_opcode!(op_memory_grow, MemoryGrow);
    impl_opcode!(op_memory_fill, MemoryFill);
    impl_opcode!(op_memory_copy, MemoryCopy);
    impl_opcode!(op_memory_init, MemoryInit(DataSegmentIdx));
    impl_opcode!(op_data_drop, DataDrop(DataSegmentIdx));
    impl_opcode!(op_table_size, TableSize(TableIdx));
    impl_opcode!(op_table_grow, TableGrow(TableIdx));
    impl_opcode!(op_table_fill, TableFill(TableIdx));
    impl_opcode!(op_table_get, TableGet(TableIdx));
    impl_opcode!(op_table_set, TableSet(TableIdx));
    impl_opcode!(op_table_copy, TableCopy(TableIdx));
    impl_opcode!(op_table_init, TableInit(ElementSegmentIdx));
    impl_opcode!(op_elem_drop, ElemDrop(ElementSegmentIdx));
    impl_opcode!(op_ref_func, RefFunc(FuncIdx));
    impl_opcode!(op_i32_const, I32Const(UntypedValue));
    impl_opcode!(op_i64_const, I64Const(UntypedValue));
    impl_opcode!(op_const_ref, ConstRef(ConstRef));
    impl_opcode!(op_i32_eqz, I32Eqz);
    impl_opcode!(op_i32_eq, I32Eq);
    impl_opcode!(op_i32_ne, I32Ne);
    impl_opcode!(op_i32_lt_s, I32LtS);
    impl_opcode!(op_i32_lt_u, I32LtU);
    impl_opcode!(op_i32_gt_s, I32GtS);
    impl_opcode!(op_i32_gt_u, I32GtU);
    impl_opcode!(op_i32_le_s, I32LeS);
    impl_opcode!(op_i32_le_u, I32LeU);
    impl_opcode!(op_i32_ge_s, I32GeS);
    impl_opcode!(op_i32_ge_u, I32GeU);
    impl_opcode!(op_i64_eqz, I64Eqz);
    impl_opcode!(op_i64_eq, I64Eq);
    impl_opcode!(op_i64_ne, I64Ne);
    impl_opcode!(op_i64_lt_s, I64LtS);
    impl_opcode!(op_i64_lt_u, I64LtU);
    impl_opcode!(op_i64_gt_s, I64GtS);
    impl_opcode!(op_i64_gt_u, I64GtU);
    impl_opcode!(op_i64_le_s, I64LeS);
    impl_opcode!(op_i64_le_u, I64LeU);
    impl_opcode!(op_i64_ge_s, I64GeS);
    impl_opcode!(op_i64_ge_u, I64GeU);
    impl_opcode!(op_f32_eq, F32Eq);
    impl_opcode!(op_f32_ne, F32Ne);
    impl_opcode!(op_f32_lt, F32Lt);
    impl_opcode!(op_f32_gt, F32Gt);
    impl_opcode!(op_f32_le, F32Le);
    impl_opcode!(op_f32_ge, F32Ge);
    impl_opcode!(op_f64_eq, F64Eq);
    impl_opcode!(op_f64_ne, F64Ne);
    impl_opcode!(op_f64_lt, F64Lt);
    impl_opcode!(op_f64_gt, F64Gt);
    impl_opcode!(op_f64_le, F64Le);
    impl_opcode!(op_f64_ge, F64Ge);
    impl_opcode!(op_i32_clz, I32Clz);
    impl_opcode!(op_i32_ctz, I32Ctz);
    impl_opcode!(op_i32_popcnt, I32Popcnt);
    impl_opcode!(op_i32_add, I32Add);
    impl_opcode!(op_i32_sub, I32Sub);
    impl_opcode!(op_i32_mul, I32Mul);
    impl_opcode!(op_i32_div_s, I32DivS);
    impl_opcode!(op_i32_div_u, I32DivU);
    impl_opcode!(op_i32_rem_s, I32RemS);
    impl_opcode!(op_i32_rem_u, I32RemU);
    impl_opcode!(op_i32_and, I32And);
    impl_opcode!(op_i32_or, I32Or);
    impl_opcode!(op_i32_xor, I32Xor);
    impl_opcode!(op_i32_shl, I32Shl);
    impl_opcode!(op_i32_shr_s, I32ShrS);
    impl_opcode!(op_i32_shr_u, I32ShrU);
    impl_opcode!(op_i32_rotl, I32Rotl);
    impl_opcode!(op_i32_rotr, I32Rotr);
    impl_opcode!(op_i64_clz, I64Clz);
    impl_opcode!(op_i64_ctz, I64Ctz);
    impl_opcode!(op_i64_popcnt, I64Popcnt);
    impl_opcode!(op_i64_add, I64Add);
    impl_opcode!(op_i64_sub, I64Sub);
    impl_opcode!(op_i64_mul, I64Mul);
    impl_opcode!(op_i64_div_s, I64DivS);
    impl_opcode!(op_i64_div_u, I64DivU);
    impl_opcode!(op_i64_rem_s, I64RemS);
    impl_opcode!(op_i64_rem_u, I64RemU);
    impl_opcode!(op_i64_and, I64And);
    impl_opcode!(op_i64_or, I64Or);
    impl_opcode!(op_i64_xor, I64Xor);
    impl_opcode!(op_i64_shl, I64Shl);
    impl_opcode!(op_i64_shr_s, I64ShrS);
    impl_opcode!(op_i64_shr_u, I64ShrU);
    impl_opcode!(op_i64_rotl, I64Rotl);
    impl_opcode!(op_i64_rotr, I64Rotr);
    impl_opcode!(op_f32_abs, F32Abs);
    impl_opcode!(op_f32_neg, F32Neg);
    impl_opcode!(op_f32_ceil, F32Ceil);
    impl_opcode!(op_f32_floor, F32Floor);
    impl_opcode!(op_f32_trunc, F32Trunc);
    impl_opcode!(op_f32_nearest, F32Nearest);
    impl_opcode!(op_f32_sqrt, F32Sqrt);
    impl_opcode!(op_f32_add, F32Add);
    impl_opcode!(op_f32_sub, F32Sub);
    impl_opcode!(op_f32_mul, F32Mul);
    impl_opcode!(op_f32_div, F32Div);
    impl_opcode!(op_f32_min, F32Min);
    impl_opcode!(op_f32_max, F32Max);
    impl_opcode!(op_f32_copysign, F32Copysign);
    impl_opcode!(op_f64_abs, F64Abs);
    impl_opcode!(op_f64_neg, F64Neg);
    impl_opcode!(op_f64_ceil, F64Ceil);
    impl_opcode!(op_f64_floor, F64Floor);
    impl_opcode!(op_f64_trunc, F64Trunc);
    impl_opcode!(op_f64_nearest, F64Nearest);
    impl_opcode!(op_f64_sqrt, F64Sqrt);
    impl_opcode!(op_f64_add, F64Add);
    impl_opcode!(op_f64_sub, F64Sub);
    impl_opcode!(op_f64_mul, F64Mul);
    impl_opcode!(op_f64_div, F64Div);
    impl_opcode!(op_f64_min, F64Min);
    impl_opcode!(op_f64_max, F64Max);
    impl_opcode!(op_f64_copysign, F64Copysign);
    impl_opcode!(op_i32_wrap_i64, I32WrapI64);
    impl_opcode!(op_i32_trunc_f32s, I32TruncF32S);
    impl_opcode!(op_i32_trunc_f32u, I32TruncF32U);
    impl_opcode!(op_i32_trunc_f64s, I32TruncF64S);
    impl_opcode!(op_i32_trunc_f64u, I32TruncF64U);
    impl_opcode!(op_i64_extend_i32s, I64ExtendI32S);
    impl_opcode!(op_i64_extend_i32u, I64ExtendI32U);
    impl_opcode!(op_i64_trunc_f32s, I64TruncF32S);
    impl_opcode!(op_i64_trunc_f32u, I64TruncF32U);
    impl_opcode!(op_i64_trunc_f64s, I64TruncF64S);
    impl_opcode!(op_i64_trunc_f64u, I64TruncF64U);
    impl_opcode!(op_f32_convert_i32s, F32ConvertI32S);
    impl_opcode!(op_f32_convert_i32u, F32ConvertI32U);
    impl_opcode!(op_f32_convert_i64s, F32ConvertI64S);
    impl_opcode!(op_f32_convert_i64u, F32ConvertI64U);
    impl_opcode!(op_f32_demote_f64, F32DemoteF64);
    impl_opcode!(op_f64_convert_i32s, F64ConvertI32S);
    impl_opcode!(op_f64_convert_i32u, F64ConvertI32U);
    impl_opcode!(op_f64_convert_i64s, F64ConvertI64S);
    impl_opcode!(op_f64_convert_i64u, F64ConvertI64U);
    impl_opcode!(op_f64_promote_f32, F64PromoteF32);
    impl_opcode!(op_i32_extend8_s, I32Extend8S);
    impl_opcode!(op_i32_extend16_s, I32Extend16S);
    impl_opcode!(op_i64_extend8_s, I64Extend8S);
    impl_opcode!(op_i64_extend16_s, I64Extend16S);
    impl_opcode!(op_i64_extend32_s, I64Extend32S);
    impl_opcode!(op_i32_trunc_sat_f32s, I32TruncSatF32S);
    impl_opcode!(op_i32_trunc_sat_f32u, I32TruncSatF32U);
    impl_opcode!(op_i32_trunc_sat_f64s, I32TruncSatF64S);
    impl_opcode!(op_i32_trunc_sat_f64u, I32TruncSatF64U);
    impl_opcode!(op_i64_trunc_sat_f32s, I64TruncSatF32S);
    impl_opcode!(op_i64_trunc_sat_f32u, I64TruncSatF32U);
    impl_opcode!(op_i64_trunc_sat_f64s, I64TruncSatF64S);
    impl_opcode!(op_i64_trunc_sat_f64u, I64TruncSatF64U);

    pub fn extend<I: Into<InstructionSet>>(&mut self, with: I) {
        self.instr.extend(Into::<InstructionSet>::into(with).instr);
    }
}

#[macro_export]
macro_rules! instruction_set_internal {
    // Nothing left to do
    ($code:ident, ) => {};
    ($code:ident, $x:ident [$v:expr] $($rest:tt)*) => {{
        $code.push(fluentbase_rwasm::engine::bytecode::Instruction::$x($v.into()));
        $crate::instruction_set_internal!($code, $($rest)*);
    }};
    ($code:ident, $x:ident ($v:expr) $($rest:tt)*) => {{
        $code.push(fluentbase_rwasm::engine::bytecode::Instruction::$x($v.into()));
        $crate::instruction_set_internal!($code, $($rest)*);
    }};
    // Default opcode without any inputs
    ($code:ident, $x:ident $($rest:tt)*) => {{
        $code.push(fluentbase_rwasm::engine::bytecode::Instruction::$x);
        $crate::instruction_set_internal!($code, $($rest)*);
    }};
    // Function calls
    ($code:ident, .$function:ident ($($args:expr),* $(,)?) $($rest:tt)*) => {{
        $code.$function($($args,)*);
        $crate::instruction_set_internal!($code, $($rest)*);
    }};
}

#[macro_export]
macro_rules! instruction_set {
    ($($args:tt)*) => {{
        let mut code = $crate::rwasm::InstructionSet::new();
        $crate::instruction_set_internal!(code, $($args)*);
        code
    }};
}

#[deprecated(note = "use [instruction_set_internal] instead")]
#[macro_export]
macro_rules! bytecode_internal {
    // Nothing left to do
    ($code:ident, ) => {};
    ($code:ident, $x:ident ($v:expr) $($rest:tt)*) => {{
        $code.$x($v);
        $crate::instruction_set_internal!($code, $($rest)*);
    }};
    // Default opcode without any inputs
    ($code:ident, $x:ident $($rest:tt)*) => {{
        $code.write_op(fluentbase_rwasm::engine::bytecode::Instruction::$x);
        $crate::instruction_set_internal!($code, $($rest)*);
    }};
    // Function calls
    ($code:ident, .$function:ident ($($args:expr),* $(,)?) $($rest:tt)*) => {{
        $code.$function($($args,)*);
        $crate::instruction_set_internal!($code, $($rest)*);
    }};
}

#[deprecated(note = "use [instruction_set] instead")]
#[macro_export]
macro_rules! bytecode {
    ($($args:tt)*) => {{
        let mut code = $crate::rwasm::InstructionSet::new();
        $crate::instruction_set_internal!(code, $($args)*);
        code
    }};
}
