use crate::binary_format::{module::decode_rwasm_module, BinaryFormatReader};
use core::fmt::Formatter;
use rwasm::{
    core::UntypedValue,
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
            LocalDepth,
            SignatureIdx,
            TableIdx,
        },
        CompiledFunc,
        DropKeep,
    },
};

#[derive(Default)]
pub struct RwasmModule2 {
    pub code_section: Vec<Instruction>,
    pub instr_data: Vec<InstructionData>,
    pub memory_section: Vec<u8>,
    pub func_section: Vec<u32>,
    pub element_section: Vec<u32>,
    pub source_pc: u32,
    pub func_segments: Vec<u32>,
}

impl RwasmModule2 {
    pub fn new_or_empty(sink: &[u8]) -> Self {
        if sink.is_empty() {
            Self::empty()
        } else {
            Self::new(sink)
        }
    }

    pub fn empty() -> Self {
        Self {
            code_section: vec![Instruction::Return],
            instr_data: vec![InstructionData::EmptyData],
            memory_section: vec![],
            func_section: vec![1],
            element_section: vec![],
            source_pc: 0,
            func_segments: vec![0],
        }
    }

    pub fn new(sink: &[u8]) -> Self {
        let mut binary_format_reader = BinaryFormatReader::new(sink);
        let mut module = decode_rwasm_module(&mut binary_format_reader)
            .unwrap_or_else(|_| unreachable!("rwasm: invalid module"));
        module.instantiate();
        module
    }

    pub fn instantiate(&mut self) {
        let mut func_segments = vec![0u32];
        let mut total_func_len = 0u32;
        for func_len in self.func_section.iter().take(self.func_section.len() - 1) {
            total_func_len += *func_len;
            func_segments.push(total_func_len);
        }
        let source_pc = func_segments
            .last()
            .copied()
            .expect("rwasm: empty function section");
        self.source_pc = source_pc;
        self.func_segments = func_segments;
    }
}

#[derive(
    Debug,
    Copy,
    Clone,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    num_enum::IntoPrimitive,
    num_enum::TryFromPrimitive,
)]
#[repr(u8)]
pub enum Instruction {
    Unreachable = 0x00,
    LocalGet = 0x01,
    LocalSet = 0x02,
    LocalTee = 0x03,
    Br = 0x04,
    BrIfEqz = 0x05,
    BrIfNez = 0x06,
    BrAdjust = 0x07,
    BrAdjustIfNez = 0x08,
    BrTable = 0x09,
    ConsumeFuel = 0x0a,
    Return = 0x0b,
    ReturnIfNez = 0x0c,
    ReturnCallInternal = 0x0d,
    ReturnCall = 0x0e,
    ReturnCallIndirect = 0x0f,
    CallInternal = 0x10,
    Call = 0x11,
    CallIndirect = 0x12,
    SignatureCheck = 0x13,
    Drop = 0x14,
    Select = 0x15,
    GlobalGet = 0x16,
    GlobalSet = 0x17,
    I32Load = 0x18,
    I64Load = 0x19,
    F32Load = 0x1a,
    F64Load = 0x1b,
    I32Load8S = 0x1c,
    I32Load8U = 0x1d,
    I32Load16S = 0x1e,
    I32Load16U = 0x1f,
    I64Load8S = 0x20,
    I64Load8U = 0x21,
    I64Load16S = 0x22,
    I64Load16U = 0x23,
    I64Load32S = 0x24,
    I64Load32U = 0x25,
    I32Store = 0x26,
    I64Store = 0x27,
    F32Store = 0x28,
    F64Store = 0x29,
    I32Store8 = 0x2a,
    I32Store16 = 0x2b,
    I64Store8 = 0x2c,
    I64Store16 = 0x2d,
    I64Store32 = 0x2e,
    MemorySize = 0x2f,
    MemoryGrow = 0x30,
    MemoryFill = 0x31,
    MemoryCopy = 0x32,
    MemoryInit = 0x33,
    DataDrop = 0x34,
    TableSize = 0x35,
    TableGrow = 0x36,
    TableFill = 0x37,
    TableGet = 0x38,
    TableSet = 0x39,
    TableCopy = 0x3a,
    TableInit = 0x3b,
    ElemDrop = 0x3c,
    RefFunc = 0x3d,
    I32Const = 0x3e,
    I64Const = 0x3f,
    F32Const = 0x40,
    F64Const = 0x41,
    I32Eqz = 0x42,
    I32Eq = 0x43,
    I32Ne = 0x44,
    I32LtS = 0x45,
    I32LtU = 0x46,
    I32GtS = 0x47,
    I32GtU = 0x48,
    I32LeS = 0x49,
    I32LeU = 0x4a,
    I32GeS = 0x4b,
    I32GeU = 0x4c,
    I64Eqz = 0x4d,
    I64Eq = 0x4e,
    I64Ne = 0x4f,
    I64LtS = 0x50,
    I64LtU = 0x51,
    I64GtS = 0x52,
    I64GtU = 0x53,
    I64LeS = 0x54,
    I64LeU = 0x55,
    I64GeS = 0x56,
    I64GeU = 0x57,
    F32Eq = 0x58,
    F32Ne = 0x59,
    F32Lt = 0x5a,
    F32Gt = 0x5b,
    F32Le = 0x5c,
    F32Ge = 0x5d,
    F64Eq = 0x5e,
    F64Ne = 0x5f,
    F64Lt = 0x60,
    F64Gt = 0x61,
    F64Le = 0x62,
    F64Ge = 0x63,
    I32Clz = 0x64,
    I32Ctz = 0x65,
    I32Popcnt = 0x66,
    I32Add = 0x67,
    I32Sub = 0x68,
    I32Mul = 0x69,
    I32DivS = 0x6a,
    I32DivU = 0x6b,
    I32RemS = 0x6c,
    I32RemU = 0x6d,
    I32And = 0x6e,
    I32Or = 0x6f,
    I32Xor = 0x70,
    I32Shl = 0x71,
    I32ShrS = 0x72,
    I32ShrU = 0x73,
    I32Rotl = 0x74,
    I32Rotr = 0x75,
    I64Clz = 0x76,
    I64Ctz = 0x77,
    I64Popcnt = 0x78,
    I64Add = 0x79,
    I64Sub = 0x7a,
    I64Mul = 0x7b,
    I64DivS = 0x7c,
    I64DivU = 0x7d,
    I64RemS = 0x7e,
    I64RemU = 0x7f,
    I64And = 0x80,
    I64Or = 0x81,
    I64Xor = 0x82,
    I64Shl = 0x83,
    I64ShrS = 0x84,
    I64ShrU = 0x85,
    I64Rotl = 0x86,
    I64Rotr = 0x87,
    F32Abs = 0x88,
    F32Neg = 0x89,
    F32Ceil = 0x8a,
    F32Floor = 0x8b,
    F32Trunc = 0x8c,
    F32Nearest = 0x8d,
    F32Sqrt = 0x8e,
    F32Add = 0x8f,
    F32Sub = 0x90,
    F32Mul = 0x91,
    F32Div = 0x92,
    F32Min = 0x93,
    F32Max = 0x94,
    F32Copysign = 0x95,
    F64Abs = 0x96,
    F64Neg = 0x97,
    F64Ceil = 0x98,
    F64Floor = 0x99,
    F64Trunc = 0x9a,
    F64Nearest = 0x9b,
    F64Sqrt = 0x9c,
    F64Add = 0x9d,
    F64Sub = 0x9e,
    F64Mul = 0x9f,
    F64Div = 0xa0,
    F64Min = 0xa1,
    F64Max = 0xa2,
    F64Copysign = 0xa3,
    I32WrapI64 = 0xa4,
    I32TruncF32S = 0xa5,
    I32TruncF32U = 0xa6,
    I32TruncF64S = 0xa7,
    I32TruncF64U = 0xa8,
    I64ExtendI32S = 0xa9,
    I64ExtendI32U = 0xaa,
    I64TruncF32S = 0xab,
    I64TruncF32U = 0xac,
    I64TruncF64S = 0xad,
    I64TruncF64U = 0xae,
    F32ConvertI32S = 0xaf,
    F32ConvertI32U = 0xb0,
    F32ConvertI64S = 0xb1,
    F32ConvertI64U = 0xb2,
    F32DemoteF64 = 0xb3,
    F64ConvertI32S = 0xb4,
    F64ConvertI32U = 0xb5,
    F64ConvertI64S = 0xb6,
    F64ConvertI64U = 0xb7,
    F64PromoteF32 = 0xb8,
    I32Extend8S = 0xb9,
    I32Extend16S = 0xba,
    I64Extend8S = 0xbb,
    I64Extend16S = 0xbc,
    I64Extend32S = 0xbd,
    I32TruncSatF32S = 0xbe,
    I32TruncSatF32U = 0xbf,
    I32TruncSatF64S = 0xc0,
    I32TruncSatF64U = 0xc1,
    I64TruncSatF32S = 0xc2,
    I64TruncSatF32U = 0xc3,
    I64TruncSatF64S = 0xc4,
    I64TruncSatF64U = 0xc5,
}

#[cfg(feature = "std")]
impl core::fmt::Display for Instruction {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        let name = format!("{:?}", self);
        let name: Vec<_> = name.split('(').collect();
        write!(f, "{}", name[0])
    }
}

#[derive(Default)]
pub enum InstructionData {
    #[default]
    EmptyData,
    LocalDepth(LocalDepth),
    BranchOffset(BranchOffset),
    BranchTableTargets(BranchTableTargets),
    BlockFuel(BlockFuel),
    DropKeep(DropKeep),
    CompiledFunc(CompiledFunc),
    FuncIdx(FuncIdx),
    SignatureIdx(SignatureIdx),
    GlobalIdx(GlobalIdx),
    AddressOffset(AddressOffset),
    DataSegmentIdx(DataSegmentIdx),
    TableIdx(TableIdx),
    ElementSegmentIdx(ElementSegmentIdx),
    UntypedValue(UntypedValue),
}
