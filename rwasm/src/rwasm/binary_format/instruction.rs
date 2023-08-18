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
            Instruction,
            LocalDepth,
            TableIdx,
        },
        CompiledFunc,
        ConstRef,
        DropKeep,
    },
    rwasm::binary_format::{
        reader_writer::{BinaryFormatReader, BinaryFormatWriter},
        BinaryFormat,
        BinaryFormatError,
    },
};
use alloc::vec::Vec;

impl<'a> BinaryFormat<'a> for Instruction {
    type SelfType = Instruction;

    fn write_binary(&self, sink: &mut BinaryFormatWriter<'a>) -> Result<usize, BinaryFormatError> {
        let n = match self {
            // local Instruction family
            Instruction::LocalGet(index) => sink.write_u8(0x00)? + index.write_binary(sink)?,
            Instruction::LocalSet(index) => sink.write_u8(0x01)? + index.write_binary(sink)?,
            Instruction::LocalTee(index) => sink.write_u8(0x02)? + index.write_binary(sink)?,
            // control flow Instruction family
            Instruction::Br(branch_params) => {
                sink.write_u8(0x03)? + branch_params.write_binary(sink)?
            }
            Instruction::BrIfEqz(branch_params) => {
                sink.write_u8(0x04)? + branch_params.write_binary(sink)?
            }
            Instruction::BrIfNez(branch_params) => {
                sink.write_u8(0x05)? + branch_params.write_binary(sink)?
            }
            Instruction::BrAdjust(branch_params) => {
                sink.write_u8(0x06)? + branch_params.write_binary(sink)?
            }
            Instruction::BrAdjustIfNez(branch_params) => {
                sink.write_u8(0x07)? + branch_params.write_binary(sink)?
            }
            Instruction::BrTable(index) => sink.write_u8(0x08)? + index.write_binary(sink)?,
            Instruction::Unreachable => sink.write_u8(0x09)?,
            Instruction::ConsumeFuel(u) => sink.write_u8(0x0a)? + u.write_binary(sink)?,
            Instruction::Return(_) => sink.write_u8(0x0b)?,
            Instruction::ReturnIfNez(_) => sink.write_u8(0x0c)?,
            Instruction::ReturnCallInternal(func) => {
                sink.write_u8(0x0d)? + func.write_binary(sink)?
            }
            Instruction::ReturnCall(func) => sink.write_u8(0x0f)? + func.write_binary(sink)?,
            // Instruction::ReturnCallIndirect(sig) => sink.write_u8(0x10)? +
            // sig.write_binary(sink)?,
            Instruction::ReturnCallIndirectUnsafe(table) => {
                sink.write_u8(0x10)? + table.write_binary(sink)?
            }
            Instruction::CallInternal(sig) => sink.write_u8(0x11)? + sig.write_binary(sink)?,
            Instruction::Call(jump_dest) => sink.write_u8(0x13)? + jump_dest.write_binary(sink)?,
            // Instruction::CallIndirect(signature) => sink.write_u8(0x14)? +
            // signature.write_binary(sink)?,
            Instruction::CallIndirectUnsafe(table) => {
                sink.write_u8(0x14)? + table.write_binary(sink)?
            }
            Instruction::Drop => sink.write_u8(0x15)?,
            Instruction::Select => sink.write_u8(0x16)?,
            // global Instruction family
            Instruction::GlobalGet(index) => sink.write_u8(0x17)? + index.write_binary(sink)?,
            Instruction::GlobalSet(index) => sink.write_u8(0x18)? + index.write_binary(sink)?,
            // memory Instruction family
            Instruction::I32Load(offset) => sink.write_u8(0x19)? + offset.write_binary(sink)?,
            Instruction::I64Load(offset) => sink.write_u8(0x1a)? + offset.write_binary(sink)?,
            Instruction::F32Load(offset) => sink.write_u8(0x1b)? + offset.write_binary(sink)?,
            Instruction::F64Load(offset) => sink.write_u8(0x1c)? + offset.write_binary(sink)?,
            Instruction::I32Load8S(offset) => sink.write_u8(0x1d)? + offset.write_binary(sink)?,
            Instruction::I32Load8U(offset) => sink.write_u8(0x1e)? + offset.write_binary(sink)?,
            Instruction::I32Load16S(offset) => sink.write_u8(0x1f)? + offset.write_binary(sink)?,
            Instruction::I32Load16U(offset) => sink.write_u8(0x20)? + offset.write_binary(sink)?,
            Instruction::I64Load8S(offset) => sink.write_u8(0x21)? + offset.write_binary(sink)?,
            Instruction::I64Load8U(offset) => sink.write_u8(0x22)? + offset.write_binary(sink)?,
            Instruction::I64Load16S(offset) => sink.write_u8(0x23)? + offset.write_binary(sink)?,
            Instruction::I64Load16U(offset) => sink.write_u8(0x24)? + offset.write_binary(sink)?,
            Instruction::I64Load32S(offset) => sink.write_u8(0x25)? + offset.write_binary(sink)?,
            Instruction::I64Load32U(offset) => sink.write_u8(0x26)? + offset.write_binary(sink)?,
            Instruction::I32Store(offset) => sink.write_u8(0x27)? + offset.write_binary(sink)?,
            Instruction::I64Store(offset) => sink.write_u8(0x28)? + offset.write_binary(sink)?,
            Instruction::F32Store(offset) => sink.write_u8(0x29)? + offset.write_binary(sink)?,
            Instruction::F64Store(offset) => sink.write_u8(0x2a)? + offset.write_binary(sink)?,
            Instruction::I32Store8(offset) => sink.write_u8(0x2b)? + offset.write_binary(sink)?,
            Instruction::I32Store16(offset) => sink.write_u8(0x2c)? + offset.write_binary(sink)?,
            Instruction::I64Store8(offset) => sink.write_u8(0x2d)? + offset.write_binary(sink)?,
            Instruction::I64Store16(offset) => sink.write_u8(0x2e)? + offset.write_binary(sink)?,
            Instruction::I64Store32(offset) => sink.write_u8(0x2f)? + offset.write_binary(sink)?,
            // memory data Instruction family (?)
            Instruction::MemorySize => sink.write_u8(0x30)?,
            Instruction::MemoryGrow => sink.write_u8(0x31)?,
            Instruction::MemoryFill => sink.write_u8(0x32)?,
            Instruction::MemoryCopy => sink.write_u8(0x33)?,
            Instruction::MemoryInit(index) => sink.write_u8(0x34)? + index.write_binary(sink)?,
            Instruction::DataDrop(index) => sink.write_u8(0x35)? + index.write_binary(sink)?,
            Instruction::TableSize(index) => sink.write_u8(0x36)? + index.write_binary(sink)?,
            Instruction::TableGrow(index) => sink.write_u8(0x37)? + index.write_binary(sink)?,
            Instruction::TableFill(index) => sink.write_u8(0x38)? + index.write_binary(sink)?,
            Instruction::TableGet(index) => sink.write_u8(0x39)? + index.write_binary(sink)?,
            Instruction::TableSet(index) => sink.write_u8(0x3a)? + index.write_binary(sink)?,
            Instruction::TableCopy(idx) => sink.write_u8(0x3b)? + idx.write_binary(sink)?,
            Instruction::TableInit(idx) => sink.write_u8(0x3c)? + idx.write_binary(sink)?,
            Instruction::ElemDrop(idx) => sink.write_u8(0x3d)? + idx.write_binary(sink)?,
            Instruction::RefFunc(idx) => sink.write_u8(0x3e)? + idx.write_binary(sink)?,
            // i32/i64 Instruction family
            Instruction::I32Const(untyped_value) => {
                sink.write_u8(0x3f)? + untyped_value.write_binary(sink)?
            }
            Instruction::I64Const(untyped_value) => {
                sink.write_u8(0x40)? + untyped_value.write_binary(sink)?
            }
            Instruction::ConstRef(const_ref) => {
                sink.write_u8(0x41)? + const_ref.write_binary(sink)?
            }
            Instruction::I32Eqz => sink.write_u8(0x42)?,
            Instruction::I32Eq => sink.write_u8(0x43)?,
            Instruction::I32Ne => sink.write_u8(0x44)?,
            Instruction::I32LtS => sink.write_u8(0x45)?,
            Instruction::I32LtU => sink.write_u8(0x46)?,
            Instruction::I32GtS => sink.write_u8(0x47)?,
            Instruction::I32GtU => sink.write_u8(0x48)?,
            Instruction::I32LeS => sink.write_u8(0x49)?,
            Instruction::I32LeU => sink.write_u8(0x4a)?,
            Instruction::I32GeS => sink.write_u8(0x4b)?,
            Instruction::I32GeU => sink.write_u8(0x4c)?,
            Instruction::I64Eqz => sink.write_u8(0x4d)?,
            Instruction::I64Eq => sink.write_u8(0x4e)?,
            Instruction::I64Ne => sink.write_u8(0x4f)?,
            Instruction::I64LtS => sink.write_u8(0x50)?,
            Instruction::I64LtU => sink.write_u8(0x51)?,
            Instruction::I64GtS => sink.write_u8(0x52)?,
            Instruction::I64GtU => sink.write_u8(0x53)?,
            Instruction::I64LeS => sink.write_u8(0x54)?,
            Instruction::I64LeU => sink.write_u8(0x55)?,
            Instruction::I64GeS => sink.write_u8(0x56)?,
            Instruction::I64GeU => sink.write_u8(0x57)?,
            Instruction::F32Eq => sink.write_u8(0x58)?,
            Instruction::F32Ne => sink.write_u8(0x59)?,
            Instruction::F32Lt => sink.write_u8(0x5a)?,
            Instruction::F32Gt => sink.write_u8(0x5b)?,
            Instruction::F32Le => sink.write_u8(0x5c)?,
            Instruction::F32Ge => sink.write_u8(0x5d)?,
            Instruction::F64Eq => sink.write_u8(0x5e)?,
            Instruction::F64Ne => sink.write_u8(0x5f)?,
            Instruction::F64Lt => sink.write_u8(0x60)?,
            Instruction::F64Gt => sink.write_u8(0x61)?,
            Instruction::F64Le => sink.write_u8(0x62)?,
            Instruction::F64Ge => sink.write_u8(0x63)?,
            Instruction::I32Clz => sink.write_u8(0x64)?,
            Instruction::I32Ctz => sink.write_u8(0x65)?,
            Instruction::I32Popcnt => sink.write_u8(0x66)?,
            Instruction::I32Add => sink.write_u8(0x67)?,
            Instruction::I32Sub => sink.write_u8(0x68)?,
            Instruction::I32Mul => sink.write_u8(0x69)?,
            Instruction::I32DivS => sink.write_u8(0x6a)?,
            Instruction::I32DivU => sink.write_u8(0x6b)?,
            Instruction::I32RemS => sink.write_u8(0x6c)?,
            Instruction::I32RemU => sink.write_u8(0x6d)?,
            Instruction::I32And => sink.write_u8(0x6e)?,
            Instruction::I32Or => sink.write_u8(0x6f)?,
            Instruction::I32Xor => sink.write_u8(0x70)?,
            Instruction::I32Shl => sink.write_u8(0x71)?,
            Instruction::I32ShrS => sink.write_u8(0x72)?,
            Instruction::I32ShrU => sink.write_u8(0x73)?,
            Instruction::I32Rotl => sink.write_u8(0x74)?,
            Instruction::I32Rotr => sink.write_u8(0x75)?,
            Instruction::I64Clz => sink.write_u8(0x76)?,
            Instruction::I64Ctz => sink.write_u8(0x77)?,
            Instruction::I64Popcnt => sink.write_u8(0x78)?,
            Instruction::I64Add => sink.write_u8(0x79)?,
            Instruction::I64Sub => sink.write_u8(0x7a)?,
            Instruction::I64Mul => sink.write_u8(0x7b)?,
            Instruction::I64DivS => sink.write_u8(0x7c)?,
            Instruction::I64DivU => sink.write_u8(0x7d)?,
            Instruction::I64RemS => sink.write_u8(0x7e)?,
            Instruction::I64RemU => sink.write_u8(0x7f)?,
            Instruction::I64And => sink.write_u8(0x80)?,
            Instruction::I64Or => sink.write_u8(0x81)?,
            Instruction::I64Xor => sink.write_u8(0x82)?,
            Instruction::I64Shl => sink.write_u8(0x83)?,
            Instruction::I64ShrS => sink.write_u8(0x84)?,
            Instruction::I64ShrU => sink.write_u8(0x85)?,
            Instruction::I64Rotl => sink.write_u8(0x86)?,
            Instruction::I64Rotr => sink.write_u8(0x87)?,
            Instruction::F32Abs => sink.write_u8(0x88)?,
            Instruction::F32Neg => sink.write_u8(0x89)?,
            Instruction::F32Ceil => sink.write_u8(0x8a)?,
            Instruction::F32Floor => sink.write_u8(0x8b)?,
            Instruction::F32Trunc => sink.write_u8(0x8c)?,
            Instruction::F32Nearest => sink.write_u8(0x8d)?,
            Instruction::F32Sqrt => sink.write_u8(0x8e)?,
            Instruction::F32Add => sink.write_u8(0x8f)?,
            Instruction::F32Sub => sink.write_u8(0x90)?,
            Instruction::F32Mul => sink.write_u8(0x91)?,
            Instruction::F32Div => sink.write_u8(0x92)?,
            Instruction::F32Min => sink.write_u8(0x93)?,
            Instruction::F32Max => sink.write_u8(0x94)?,
            Instruction::F32Copysign => sink.write_u8(0x95)?,
            Instruction::F64Abs => sink.write_u8(0x96)?,
            Instruction::F64Neg => sink.write_u8(0x97)?,
            Instruction::F64Ceil => sink.write_u8(0x98)?,
            Instruction::F64Floor => sink.write_u8(0x99)?,
            Instruction::F64Trunc => sink.write_u8(0x9a)?,
            Instruction::F64Nearest => sink.write_u8(0x9b)?,
            Instruction::F64Sqrt => sink.write_u8(0x9c)?,
            Instruction::F64Add => sink.write_u8(0x9d)?,
            Instruction::F64Sub => sink.write_u8(0x9e)?,
            Instruction::F64Mul => sink.write_u8(0x9f)?,
            Instruction::F64Div => sink.write_u8(0xa0)?,
            Instruction::F64Min => sink.write_u8(0xa1)?,
            Instruction::F64Max => sink.write_u8(0xa2)?,
            Instruction::F64Copysign => sink.write_u8(0xa3)?,
            Instruction::I32WrapI64 => sink.write_u8(0xa4)?,
            Instruction::I32TruncF32S => sink.write_u8(0xa5)?,
            Instruction::I32TruncF32U => sink.write_u8(0xa6)?,
            Instruction::I32TruncF64S => sink.write_u8(0xa7)?,
            Instruction::I32TruncF64U => sink.write_u8(0xa8)?,
            Instruction::I64ExtendI32S => sink.write_u8(0xa9)?,
            Instruction::I64ExtendI32U => sink.write_u8(0xaa)?,
            Instruction::I64TruncF32S => sink.write_u8(0xab)?,
            Instruction::I64TruncF32U => sink.write_u8(0xac)?,
            Instruction::I64TruncF64S => sink.write_u8(0xad)?,
            Instruction::I64TruncF64U => sink.write_u8(0xae)?,
            Instruction::F32ConvertI32S => sink.write_u8(0xaf)?,
            Instruction::F32ConvertI32U => sink.write_u8(0xb0)?,
            Instruction::F32ConvertI64S => sink.write_u8(0xb1)?,
            Instruction::F32ConvertI64U => sink.write_u8(0xb2)?,
            Instruction::F32DemoteF64 => sink.write_u8(0xb3)?,
            Instruction::F64ConvertI32S => sink.write_u8(0xb4)?,
            Instruction::F64ConvertI32U => sink.write_u8(0xb5)?,
            Instruction::F64ConvertI64S => sink.write_u8(0xb6)?,
            Instruction::F64ConvertI64U => sink.write_u8(0xb7)?,
            Instruction::F64PromoteF32 => sink.write_u8(0xb8)?,
            Instruction::I32Extend8S => sink.write_u8(0xb9)?,
            Instruction::I32Extend16S => sink.write_u8(0xba)?,
            Instruction::I64Extend8S => sink.write_u8(0xbb)?,
            Instruction::I64Extend16S => sink.write_u8(0xbc)?,
            Instruction::I64Extend32S => sink.write_u8(0xbd)?,
            Instruction::I32TruncSatF32S => sink.write_u8(0xbe)?,
            Instruction::I32TruncSatF32U => sink.write_u8(0xbf)?,
            Instruction::I32TruncSatF64S => sink.write_u8(0xc0)?,
            Instruction::I32TruncSatF64U => sink.write_u8(0xc1)?,
            Instruction::I64TruncSatF32S => sink.write_u8(0xc2)?,
            Instruction::I64TruncSatF32U => sink.write_u8(0xc3)?,
            Instruction::I64TruncSatF64S => sink.write_u8(0xc4)?,
            Instruction::I64TruncSatF64U => sink.write_u8(0xc5)?,
            Instruction::SanitizerStackCheck(note) => {
                sink.write_u8(0xc6)? + note.write_binary(sink)?
            }
            _ => unreachable!("not supported opcode: {:?}", self),
        };
        Ok(n)
    }

    fn read_binary(sink: &mut BinaryFormatReader<'a>) -> Result<Instruction, BinaryFormatError> {
        let byte = sink.read_u8()?;
        Ok(match byte {
            // local Instruction family
            0x00 => Instruction::LocalGet(LocalDepth::read_binary(sink)?),
            0x01 => Instruction::LocalSet(LocalDepth::read_binary(sink)?),
            0x02 => Instruction::LocalTee(LocalDepth::read_binary(sink)?),
            // control flow Instruction family
            0x03 => Instruction::Br(BranchOffset::read_binary(sink)?),
            0x04 => Instruction::BrIfEqz(BranchOffset::read_binary(sink)?),
            0x05 => Instruction::BrIfNez(BranchOffset::read_binary(sink)?),
            0x06 => Instruction::BrAdjust(BranchOffset::read_binary(sink)?),
            0x07 => Instruction::BrAdjustIfNez(BranchOffset::read_binary(sink)?),
            0x08 => Instruction::BrTable(BranchTableTargets::read_binary(sink)?),
            0x09 => Instruction::Unreachable,
            0x0a => Instruction::ConsumeFuel(BlockFuel::read_binary(sink)?),
            0x0b => Instruction::Return(DropKeep::none()),
            0x0c => Instruction::ReturnIfNez(DropKeep::none()),
            0x0d => Instruction::ReturnCallInternal(CompiledFunc::read_binary(sink)?),
            0x0f => Instruction::ReturnCall(FuncIdx::read_binary(sink)?),
            // 0x10 => Instruction::ReturnCallIndirect(SignatureIdx::read_binary(sink)?),
            0x10 => Instruction::ReturnCallIndirectUnsafe(TableIdx::read_binary(sink)?),
            0x11 => Instruction::CallInternal(CompiledFunc::read_binary(sink)?),
            0x13 => Instruction::Call(FuncIdx::read_binary(sink)?),
            // 0x14 => Instruction::CallIndirect(SignatureIdx::read_binary(sink)?),
            0x14 => Instruction::CallIndirectUnsafe(TableIdx::read_binary(sink)?),
            0x15 => Instruction::Drop,
            0x16 => Instruction::Select,
            // global Instruction family
            0x17 => Instruction::GlobalGet(GlobalIdx::read_binary(sink)?),
            0x18 => Instruction::GlobalSet(GlobalIdx::read_binary(sink)?),
            // memory Instruction family
            0x19 => Instruction::I32Load(AddressOffset::read_binary(sink)?),
            0x1a => Instruction::I64Load(AddressOffset::read_binary(sink)?),
            0x1b => Instruction::F32Load(AddressOffset::read_binary(sink)?),
            0x1c => Instruction::F64Load(AddressOffset::read_binary(sink)?),
            0x1d => Instruction::I32Load8S(AddressOffset::read_binary(sink)?),
            0x1e => Instruction::I32Load8U(AddressOffset::read_binary(sink)?),
            0x1f => Instruction::I32Load16S(AddressOffset::read_binary(sink)?),
            0x20 => Instruction::I32Load16U(AddressOffset::read_binary(sink)?),
            0x21 => Instruction::I64Load8S(AddressOffset::read_binary(sink)?),
            0x22 => Instruction::I64Load8U(AddressOffset::read_binary(sink)?),
            0x23 => Instruction::I64Load16S(AddressOffset::read_binary(sink)?),
            0x24 => Instruction::I64Load16U(AddressOffset::read_binary(sink)?),
            0x25 => Instruction::I64Load32S(AddressOffset::read_binary(sink)?),
            0x26 => Instruction::I64Load32U(AddressOffset::read_binary(sink)?),
            0x27 => Instruction::I32Store(AddressOffset::read_binary(sink)?),
            0x28 => Instruction::I64Store(AddressOffset::read_binary(sink)?),
            0x29 => Instruction::F32Store(AddressOffset::read_binary(sink)?),
            0x2a => Instruction::F64Store(AddressOffset::read_binary(sink)?),
            0x2b => Instruction::I32Store8(AddressOffset::read_binary(sink)?),
            0x2c => Instruction::I32Store16(AddressOffset::read_binary(sink)?),
            0x2d => Instruction::I64Store8(AddressOffset::read_binary(sink)?),
            0x2e => Instruction::I64Store16(AddressOffset::read_binary(sink)?),
            0x2f => Instruction::I64Store32(AddressOffset::read_binary(sink)?),
            // memory data Instruction family (?)
            0x30 => Instruction::MemorySize,
            0x31 => Instruction::MemoryGrow,
            0x32 => Instruction::MemoryFill,
            0x33 => Instruction::MemoryCopy,
            0x34 => Instruction::MemoryInit(DataSegmentIdx::read_binary(sink)?),
            0x35 => Instruction::DataDrop(DataSegmentIdx::read_binary(sink)?),
            0x36 => Instruction::TableSize(TableIdx::read_binary(sink)?),
            0x37 => Instruction::TableGrow(TableIdx::read_binary(sink)?),
            0x38 => Instruction::TableFill(TableIdx::read_binary(sink)?),
            0x39 => Instruction::TableGet(TableIdx::read_binary(sink)?),
            0x3a => Instruction::TableSet(TableIdx::read_binary(sink)?),
            0x3b => Instruction::TableCopy(TableIdx::read_binary(sink)?),
            0x3c => Instruction::TableInit(ElementSegmentIdx::read_binary(sink)?),
            0x3d => Instruction::ElemDrop(ElementSegmentIdx::read_binary(sink)?),
            0x3e => Instruction::RefFunc(FuncIdx::read_binary(sink)?),
            // i32/i64 Instruction family
            0x3f => Instruction::I32Const(UntypedValue::read_binary(sink)?),
            0x40 => Instruction::I64Const(UntypedValue::read_binary(sink)?),
            0x41 => Instruction::ConstRef(ConstRef::read_binary(sink)?),
            0x42 => Instruction::I32Eqz,
            0x43 => Instruction::I32Eq,
            0x44 => Instruction::I32Ne,
            0x45 => Instruction::I32LtS,
            0x46 => Instruction::I32LtU,
            0x47 => Instruction::I32GtS,
            0x48 => Instruction::I32GtU,
            0x49 => Instruction::I32LeS,
            0x4a => Instruction::I32LeU,
            0x4b => Instruction::I32GeS,
            0x4c => Instruction::I32GeU,
            0x4d => Instruction::I64Eqz,
            0x4e => Instruction::I64Eq,
            0x4f => Instruction::I64Ne,
            0x50 => Instruction::I64LtS,
            0x51 => Instruction::I64LtU,
            0x52 => Instruction::I64GtS,
            0x53 => Instruction::I64GtU,
            0x54 => Instruction::I64LeS,
            0x55 => Instruction::I64LeU,
            0x56 => Instruction::I64GeS,
            0x57 => Instruction::I64GeU,
            0x58 => Instruction::F32Eq,
            0x59 => Instruction::F32Ne,
            0x5a => Instruction::F32Lt,
            0x5b => Instruction::F32Gt,
            0x5c => Instruction::F32Le,
            0x5d => Instruction::F32Ge,
            0x5e => Instruction::F64Eq,
            0x5f => Instruction::F64Ne,
            0x60 => Instruction::F64Lt,
            0x61 => Instruction::F64Gt,
            0x62 => Instruction::F64Le,
            0x63 => Instruction::F64Ge,
            0x64 => Instruction::I32Clz,
            0x65 => Instruction::I32Ctz,
            0x66 => Instruction::I32Popcnt,
            0x67 => Instruction::I32Add,
            0x68 => Instruction::I32Sub,
            0x69 => Instruction::I32Mul,
            0x6a => Instruction::I32DivS,
            0x6b => Instruction::I32DivU,
            0x6c => Instruction::I32RemS,
            0x6d => Instruction::I32RemU,
            0x6e => Instruction::I32And,
            0x6f => Instruction::I32Or,
            0x70 => Instruction::I32Xor,
            0x71 => Instruction::I32Shl,
            0x72 => Instruction::I32ShrS,
            0x73 => Instruction::I32ShrU,
            0x74 => Instruction::I32Rotl,
            0x75 => Instruction::I32Rotr,
            0x76 => Instruction::I64Clz,
            0x77 => Instruction::I64Ctz,
            0x78 => Instruction::I64Popcnt,
            0x79 => Instruction::I64Add,
            0x7a => Instruction::I64Sub,
            0x7b => Instruction::I64Mul,
            0x7c => Instruction::I64DivS,
            0x7d => Instruction::I64DivU,
            0x7e => Instruction::I64RemS,
            0x7f => Instruction::I64RemU,
            0x80 => Instruction::I64And,
            0x81 => Instruction::I64Or,
            0x82 => Instruction::I64Xor,
            0x83 => Instruction::I64Shl,
            0x84 => Instruction::I64ShrS,
            0x85 => Instruction::I64ShrU,
            0x86 => Instruction::I64Rotl,
            0x87 => Instruction::I64Rotr,
            0x88 => Instruction::F32Abs,
            0x89 => Instruction::F32Neg,
            0x8a => Instruction::F32Ceil,
            0x8b => Instruction::F32Floor,
            0x8c => Instruction::F32Trunc,
            0x8d => Instruction::F32Nearest,
            0x8e => Instruction::F32Sqrt,
            0x8f => Instruction::F32Add,
            0x90 => Instruction::F32Sub,
            0x91 => Instruction::F32Mul,
            0x92 => Instruction::F32Div,
            0x93 => Instruction::F32Min,
            0x94 => Instruction::F32Max,
            0x95 => Instruction::F32Copysign,
            0x96 => Instruction::F64Abs,
            0x97 => Instruction::F64Neg,
            0x98 => Instruction::F64Ceil,
            0x99 => Instruction::F64Floor,
            0x9a => Instruction::F64Trunc,
            0x9b => Instruction::F64Nearest,
            0x9c => Instruction::F64Sqrt,
            0x9d => Instruction::F64Add,
            0x9e => Instruction::F64Sub,
            0x9f => Instruction::F64Mul,
            0xa0 => Instruction::F64Div,
            0xa1 => Instruction::F64Min,
            0xa2 => Instruction::F64Max,
            0xa3 => Instruction::F64Copysign,
            0xa4 => Instruction::I32WrapI64,
            0xa5 => Instruction::I32TruncF32S,
            0xa6 => Instruction::I32TruncF32U,
            0xa7 => Instruction::I32TruncF64S,
            0xa8 => Instruction::I32TruncF64U,
            0xa9 => Instruction::I64ExtendI32S,
            0xaa => Instruction::I64ExtendI32U,
            0xab => Instruction::I64TruncF32S,
            0xac => Instruction::I64TruncF32U,
            0xad => Instruction::I64TruncF64S,
            0xae => Instruction::I64TruncF64U,
            0xaf => Instruction::F32ConvertI32S,
            0xb0 => Instruction::F32ConvertI32U,
            0xb1 => Instruction::F32ConvertI64S,
            0xb2 => Instruction::F32ConvertI64U,
            0xb3 => Instruction::F32DemoteF64,
            0xb4 => Instruction::F64ConvertI32S,
            0xb5 => Instruction::F64ConvertI32U,
            0xb6 => Instruction::F64ConvertI64S,
            0xb7 => Instruction::F64ConvertI64U,
            0xb8 => Instruction::F64PromoteF32,
            0xb9 => Instruction::I32Extend8S,
            0xba => Instruction::I32Extend16S,
            0xbb => Instruction::I64Extend8S,
            0xbc => Instruction::I64Extend16S,
            0xbd => Instruction::I64Extend32S,
            0xbe => Instruction::I32TruncSatF32S,
            0xbf => Instruction::I32TruncSatF32U,
            0xc0 => Instruction::I32TruncSatF64S,
            0xc1 => Instruction::I32TruncSatF64U,
            0xc2 => Instruction::I64TruncSatF32S,
            0xc3 => Instruction::I64TruncSatF32U,
            0xc4 => Instruction::I64TruncSatF64S,
            0xc5 => Instruction::I64TruncSatF64U,

            0xc6 => Instruction::SanitizerStackCheck(i32::read_binary(sink)?),

            _ => return Err(BinaryFormatError::IllegalOpcode(byte)),
        })
    }
}

impl Instruction {
    pub fn is_supported(&self) -> bool {
        match self {
            Instruction::CallIndirect(_) | Instruction::ReturnCallIndirect(_) => false,
            _ => true,
        }
    }

    pub fn aux_value(&self) -> Option<UntypedValue> {
        let value: UntypedValue = match self {
            Instruction::LocalGet(val)
            | Instruction::LocalSet(val)
            | Instruction::LocalTee(val) => val.to_usize().into(),
            Instruction::Br(val)
            | Instruction::BrIfEqz(val)
            | Instruction::BrIfNez(val)
            | Instruction::BrAdjust(val)
            | Instruction::BrAdjustIfNez(val) => val.to_i32().into(),
            Instruction::BrTable(val) => val.to_usize().into(),
            Instruction::ConsumeFuel(val) => val.to_u64().into(),
            Instruction::ReturnCallInternal(val) | Instruction::CallInternal(val) => {
                val.to_u32().into()
            }
            Instruction::ReturnCall(val) | Instruction::Call(val) => val.to_u32().into(),
            Instruction::ReturnCallIndirect(val) | Instruction::CallIndirect(val) => {
                val.to_u32().into()
            }
            Instruction::ReturnCallIndirectUnsafe(val) | Instruction::CallIndirectUnsafe(val) => {
                val.to_u32().into()
            }
            Instruction::GlobalGet(val) | Instruction::GlobalSet(val) => val.to_u32().into(),
            Instruction::I32Load(val)
            | Instruction::I64Load(val)
            | Instruction::F32Load(val)
            | Instruction::F64Load(val)
            | Instruction::I32Load8S(val)
            | Instruction::I32Load8U(val)
            | Instruction::I32Load16S(val)
            | Instruction::I32Load16U(val)
            | Instruction::I64Load8S(val)
            | Instruction::I64Load8U(val)
            | Instruction::I64Load16S(val)
            | Instruction::I64Load16U(val)
            | Instruction::I64Load32S(val)
            | Instruction::I64Load32U(val)
            | Instruction::I32Store(val)
            | Instruction::I64Store(val)
            | Instruction::F32Store(val)
            | Instruction::F64Store(val)
            | Instruction::I32Store8(val)
            | Instruction::I32Store16(val)
            | Instruction::I64Store8(val)
            | Instruction::I64Store16(val)
            | Instruction::I64Store32(val) => val.into_inner().into(),
            Instruction::MemoryInit(val) | Instruction::DataDrop(val) => val.to_u32().into(),
            Instruction::TableSize(val)
            | Instruction::TableGrow(val)
            | Instruction::TableFill(val)
            | Instruction::TableGet(val)
            | Instruction::TableSet(val)
            | Instruction::TableCopy(val) => val.to_u32().into(),
            Instruction::TableInit(val) | Instruction::ElemDrop(val) => val.to_u32().into(),
            Instruction::RefFunc(val) => val.to_u32().into(),
            Instruction::I32Const(val) | Instruction::I64Const(val) => *val,
            Instruction::ConstRef(val) => val.to_usize().into(),
            Instruction::SanitizerStackCheck(val) => (*val).into(),
            _ => return None,
        };
        Some(value)
    }

    pub fn info(&self) -> (u8, usize) {
        let mut sink: Vec<u8> = vec![0; 100];
        let mut binary_writer = BinaryFormatWriter::new(sink.as_mut_slice());
        let size = self.write_binary(&mut binary_writer).unwrap();
        (sink[0], size - 1)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        engine::{bytecode::Instruction, CompiledFunc},
        rwasm::binary_format::{
            reader_writer::{BinaryFormatReader, BinaryFormatWriter},
            BinaryFormat,
        },
    };
    use alloc::vec::Vec;
    use strum::IntoEnumIterator;

    #[test]
    fn test_opcode_encoding() {
        for opcode in Instruction::iter() {
            if !opcode.is_supported() {
                continue;
            }
            let mut buf = vec![0; 100];
            let mut writer = BinaryFormatWriter::new(buf.as_mut_slice());
            if opcode.write_binary(&mut writer).unwrap() == 0 {
                continue;
            }
            let (first_byte, _hint_size) = opcode.info();
            assert_eq!(first_byte, buf[0]);
            let mut reader = BinaryFormatReader::new(buf.as_slice());
            let opcode2 = Instruction::read_binary(&mut reader).unwrap();
            assert_eq!(opcode, opcode2);
        }
    }

    #[test]
    fn test_call_internal_encoding() {
        let opcode = Instruction::CallInternal(CompiledFunc::from(7));
        let mut buff = Vec::with_capacity(100);
        opcode.write_binary_to_vec(&mut buff).unwrap();
        let mut binary_reader = BinaryFormatReader::new(buff.as_slice());
        let opcode2 = Instruction::read_binary(&mut binary_reader).unwrap();
        assert_eq!(opcode, opcode2)
    }
}
