use crate::common::UntypedValue;
use crate::engine::bytecode::{
    AddressOffset, BlockFuel, BranchOffset, BranchTableTargets, DataSegmentIdx, ElementSegmentIdx, FuncIdx, GlobalIdx,
    Instruction, LocalDepth, SignatureIdx, TableIdx,
};
use crate::engine::DropKeep;
use crate::rwasm::binary_format::reader_writer::{BinaryFormatReader, BinaryFormatWriter};
use crate::rwasm::binary_format::{BinaryFormat, BinaryFormatError};

impl<'a> BinaryFormat<'a> for Instruction {
    type SelfType = Instruction;

    fn write_binary(&self, sink: &mut BinaryFormatWriter<'a>) -> Result<(), BinaryFormatError> {
        match self {
            Instruction::Unreachable => sink.write_u8(0x00)?,
            Instruction::ConsumeFuel(u) => {
                sink.write_u8(0x01)?;
                u.write_binary(sink)?;
            }
            Instruction::Drop => sink.write_u8(0x02)?,
            Instruction::Select => sink.write_u8(0x04)?,

            // local Instruction family
            Instruction::LocalGet(index) => {
                sink.write_u8(0x10)?;
                index.write_binary(sink)?;
            }
            Instruction::LocalSet(index) => {
                sink.write_u8(0x11)?;
                index.write_binary(sink)?;
            }
            Instruction::LocalTee(index) => {
                sink.write_u8(0x12)?;
                index.write_binary(sink)?;
            }

            // control flow Instruction family
            Instruction::Br(branch_params) => {
                sink.write_u8(0x20)?;
                branch_params.write_binary(sink)?;
            }
            Instruction::BrIfEqz(branch_params) => {
                sink.write_u8(0x21)?;
                branch_params.write_binary(sink)?;
            }
            Instruction::BrIfNez(branch_params) => {
                sink.write_u8(0x22)?;
                branch_params.write_binary(sink)?;
            }
            Instruction::BrTable(index) => {
                sink.write_u8(0x23)?;
                index.write_binary(sink)?;
            }
            Instruction::Return(drop_keep) => {
                sink.write_u8(0x24)?;
            }
            Instruction::ReturnIfNez(drop_keep) => {
                sink.write_u8(0x25)?;
            }
            Instruction::ReturnCall(func) => {
                sink.write_u8(0x26)?;
                func.write_binary(sink)?;
            }
            Instruction::ReturnCallIndirect(sig) => {
                sink.write_u8(0x27)?;
                sig.write_binary(sink)?;
            }
            Instruction::Call(jump_dest) => {
                sink.write_u8(0x28)?;
                jump_dest.write_binary(sink)?;
            }
            Instruction::CallIndirect(table) => {
                sink.write_u8(0x2A)?;
                table.write_binary(sink)?;
            }

            // global Instruction family
            Instruction::GlobalGet(index) => {
                sink.write_u8(0x30)?;
                index.write_binary(sink)?;
            }
            Instruction::GlobalSet(index) => {
                sink.write_u8(0x31)?;
                index.write_binary(sink)?;
            }

            // memory Instruction family
            Instruction::I32Load(offset) => {
                sink.write_u8(0x40)?;
                offset.write_binary(sink)?;
            }
            Instruction::I64Load(offset) => {
                sink.write_u8(0x41)?;
                offset.write_binary(sink)?;
            }
            Instruction::I32Load8S(offset) => {
                sink.write_u8(0x42)?;
                offset.write_binary(sink)?;
            }
            Instruction::I32Load8U(offset) => {
                sink.write_u8(0x43)?;
                offset.write_binary(sink)?;
            }
            Instruction::I32Load16S(offset) => {
                sink.write_u8(0x44)?;
                offset.write_binary(sink)?;
            }
            Instruction::I32Load16U(offset) => {
                sink.write_u8(0x45)?;
                offset.write_binary(sink)?;
            }
            Instruction::I64Load8S(offset) => {
                sink.write_u8(0x46)?;
                offset.write_binary(sink)?;
            }
            Instruction::I64Load8U(offset) => {
                sink.write_u8(0x47)?;
                offset.write_binary(sink)?;
            }
            Instruction::I64Load16S(offset) => {
                sink.write_u8(0x48)?;
                offset.write_binary(sink)?;
            }
            Instruction::I64Load16U(offset) => {
                sink.write_u8(0x49)?;
                offset.write_binary(sink)?;
            }
            Instruction::I64Load32S(offset) => {
                sink.write_u8(0x4A)?;
                offset.write_binary(sink)?;
            }
            Instruction::I64Load32U(offset) => {
                sink.write_u8(0x4B)?;
                offset.write_binary(sink)?;
            }
            Instruction::I32Store(offset) => {
                sink.write_u8(0x4C)?;
                offset.write_binary(sink)?;
            }
            Instruction::I64Store(offset) => {
                sink.write_u8(0x4D)?;
                offset.write_binary(sink)?;
            }
            Instruction::I32Store8(offset) => {
                sink.write_u8(0x4E)?;
                offset.write_binary(sink)?;
            }
            Instruction::I32Store16(offset) => {
                sink.write_u8(0x4F)?;
                offset.write_binary(sink)?;
            }
            Instruction::I64Store8(offset) => {
                sink.write_u8(0x50)?;
                offset.write_binary(sink)?;
            }
            Instruction::I64Store16(offset) => {
                sink.write_u8(0x51)?;
                offset.write_binary(sink)?;
            }
            Instruction::I64Store32(offset) => {
                sink.write_u8(0x52)?;
                offset.write_binary(sink)?;
            }

            // memory data Instruction family (?)
            Instruction::MemorySize => sink.write_u8(0x53)?,
            Instruction::MemoryGrow => sink.write_u8(0x54)?,
            Instruction::MemoryFill => sink.write_u8(0x55)?,
            Instruction::MemoryCopy => sink.write_u8(0x56)?,
            Instruction::MemoryInit(index) => {
                sink.write_u8(0x57)?;
                index.write_binary(sink)?;
            }
            Instruction::DataDrop(index) => {
                sink.write_u8(0x58)?;
                index.write_binary(sink)?;
            }
            Instruction::TableSize(index) => {
                sink.write_u8(0x59)?;
                index.write_binary(sink)?;
            }
            Instruction::TableGrow(index) => {
                sink.write_u8(0x5a)?;
                index.write_binary(sink)?;
            }
            Instruction::TableFill(index) => {
                sink.write_u8(0x5b)?;
                index.write_binary(sink)?;
            }
            Instruction::TableGet(index) => {
                sink.write_u8(0x5c)?;
                index.write_binary(sink)?;
            }
            Instruction::TableSet(index) => {
                sink.write_u8(0x5d)?;
                index.write_binary(sink)?;
            }
            Instruction::TableCopy(idx) => {
                sink.write_u8(0x5e)?;
                idx.write_binary(sink)?;
            }
            Instruction::TableInit(idx) => {
                sink.write_u8(0x5f)?;
                idx.write_binary(sink)?;
            }
            // Instruction::ElemDrop(index) => {
            //     sink.write_u8(0x60)?;
            //     index.write_binary(sink)?;
            // }
            // Instruction::RefFunc(index) => {
            //     sink.write_u8(0x61)?;
            //     index.write_binary(sink)?;
            // }

            // i32/i64 Instruction family
            Instruction::I64Const(untyped_value) => {
                sink.write_u8(0x60)?;
                untyped_value.write_binary(sink)?;
            }
            Instruction::I32Const(untyped_value) => {
                sink.write_u8(0x61)?;
                untyped_value.write_binary(sink)?;
            }
            Instruction::I32Eqz => sink.write_u8(0x62)?,
            Instruction::I32Eq => sink.write_u8(0x63)?,
            Instruction::I32Ne => sink.write_u8(0x64)?,
            Instruction::I32LtS => sink.write_u8(0x65)?,
            Instruction::I32LtU => sink.write_u8(0x66)?,
            Instruction::I32GtS => sink.write_u8(0x67)?,
            Instruction::I32GtU => sink.write_u8(0x68)?,
            Instruction::I32LeS => sink.write_u8(0x69)?,
            Instruction::I32LeU => sink.write_u8(0x6A)?,
            Instruction::I32GeS => sink.write_u8(0x6B)?,
            Instruction::I32GeU => sink.write_u8(0x6C)?,
            Instruction::I64Eqz => sink.write_u8(0x6D)?,
            Instruction::I64Eq => sink.write_u8(0x6E)?,
            Instruction::I64Ne => sink.write_u8(0x6F)?,
            Instruction::I64LtS => sink.write_u8(0x70)?,
            Instruction::I64LtU => sink.write_u8(0x71)?,
            Instruction::I64GtS => sink.write_u8(0x72)?,
            Instruction::I64GtU => sink.write_u8(0x73)?,
            Instruction::I64LeS => sink.write_u8(0x74)?,
            Instruction::I64LeU => sink.write_u8(0x75)?,
            Instruction::I64GeS => sink.write_u8(0x76)?,
            Instruction::I64GeU => sink.write_u8(0x77)?,
            Instruction::I32Clz => sink.write_u8(0x78)?,
            Instruction::I32Ctz => sink.write_u8(0x79)?,
            Instruction::I32Popcnt => sink.write_u8(0x7A)?,
            Instruction::I32Add => sink.write_u8(0x7B)?,
            Instruction::I32Sub => sink.write_u8(0x7C)?,
            Instruction::I32Mul => sink.write_u8(0x7D)?,
            Instruction::I32DivS => sink.write_u8(0x7E)?,
            Instruction::I32DivU => sink.write_u8(0x7F)?,
            Instruction::I32RemS => sink.write_u8(0x80)?,
            Instruction::I32RemU => sink.write_u8(0x81)?,
            Instruction::I32And => sink.write_u8(0x82)?,
            Instruction::I32Or => sink.write_u8(0x83)?,
            Instruction::I32Xor => sink.write_u8(0x84)?,
            Instruction::I32Shl => sink.write_u8(0x85)?,
            Instruction::I32ShrS => sink.write_u8(0x86)?,
            Instruction::I32ShrU => sink.write_u8(0x87)?,
            Instruction::I32Rotl => sink.write_u8(0x88)?,
            Instruction::I32Rotr => sink.write_u8(0x89)?,
            Instruction::I64Clz => sink.write_u8(0x8A)?,
            Instruction::I64Ctz => sink.write_u8(0x8B)?,
            Instruction::I64Popcnt => sink.write_u8(0x8C)?,
            Instruction::I64Add => sink.write_u8(0x8D)?,
            Instruction::I64Sub => sink.write_u8(0x8E)?,
            Instruction::I64Mul => sink.write_u8(0x8F)?,
            Instruction::I64DivS => sink.write_u8(0x90)?,
            Instruction::I64DivU => sink.write_u8(0x91)?,
            Instruction::I64RemS => sink.write_u8(0x92)?,
            Instruction::I64RemU => sink.write_u8(0x93)?,
            Instruction::I64And => sink.write_u8(0x94)?,
            Instruction::I64Or => sink.write_u8(0x95)?,
            Instruction::I64Xor => sink.write_u8(0x96)?,
            Instruction::I64Shl => sink.write_u8(0x97)?,
            Instruction::I64ShrS => sink.write_u8(0x98)?,
            Instruction::I64ShrU => sink.write_u8(0x99)?,
            Instruction::I64Rotl => sink.write_u8(0x9A)?,
            Instruction::I64Rotr => sink.write_u8(0x9B)?,
            Instruction::I32WrapI64 => sink.write_u8(0x9C)?,
            Instruction::I64ExtendI32S => sink.write_u8(0x9D)?,
            Instruction::I64ExtendI32U => sink.write_u8(0x9E)?,
            Instruction::I32Extend8S => sink.write_u8(0x9F)?,
            Instruction::I32Extend16S => sink.write_u8(0xA0)?,
            Instruction::I64Extend8S => sink.write_u8(0xA1)?,
            Instruction::I64Extend16S => sink.write_u8(0xA2)?,
            Instruction::I64Extend32S => sink.write_u8(0xA3)?,

            _ => return Ok(()),
        }
        Ok(())
    }

    fn read_binary(sink: &mut BinaryFormatReader<'a>) -> Result<Instruction, BinaryFormatError> {
        let byte = sink.read_u8()?;
        Ok(match byte {
            0x00 => Instruction::Unreachable,
            0x01 => Instruction::ConsumeFuel(BlockFuel::read_binary(sink)?),
            0x02 => Instruction::Drop,
            0x04 => Instruction::Select,

            // local Instruction family
            0x10 => Instruction::LocalGet(LocalDepth::read_binary(sink)?),
            0x11 => Instruction::LocalSet(LocalDepth::read_binary(sink)?),
            0x12 => Instruction::LocalTee(LocalDepth::read_binary(sink)?),

            // control flow Instruction family
            0x20 => Instruction::Br(BranchOffset::read_binary(sink)?),
            0x21 => Instruction::BrIfEqz(BranchOffset::read_binary(sink)?),
            0x22 => Instruction::BrIfNez(BranchOffset::read_binary(sink)?),
            0x23 => Instruction::BrTable(BranchTableTargets::read_binary(sink)?),
            0x24 => Instruction::Return(DropKeep::none()),
            0x25 => Instruction::ReturnIfNez(DropKeep::none()),
            0x26 => Instruction::ReturnCall(FuncIdx::read_binary(sink)?),
            0x27 => Instruction::ReturnCallIndirect(SignatureIdx::read_binary(sink)?),
            0x28 => Instruction::Call(FuncIdx::read_binary(sink)?),
            0x2A => Instruction::CallIndirect(SignatureIdx::read_binary(sink)?),

            // global Instruction family
            0x30 => Instruction::GlobalGet(GlobalIdx::read_binary(sink)?),
            0x31 => Instruction::GlobalSet(GlobalIdx::read_binary(sink)?),

            // memory Instruction family
            0x40 => Instruction::I32Load(AddressOffset::read_binary(sink)?),
            0x41 => Instruction::I64Load(AddressOffset::read_binary(sink)?),
            0x42 => Instruction::I32Load8S(AddressOffset::read_binary(sink)?),
            0x43 => Instruction::I32Load8U(AddressOffset::read_binary(sink)?),
            0x44 => Instruction::I32Load16S(AddressOffset::read_binary(sink)?),
            0x45 => Instruction::I32Load16U(AddressOffset::read_binary(sink)?),
            0x46 => Instruction::I64Load8S(AddressOffset::read_binary(sink)?),
            0x47 => Instruction::I64Load8U(AddressOffset::read_binary(sink)?),
            0x48 => Instruction::I64Load16S(AddressOffset::read_binary(sink)?),
            0x49 => Instruction::I64Load16U(AddressOffset::read_binary(sink)?),
            0x4A => Instruction::I64Load32S(AddressOffset::read_binary(sink)?),
            0x4B => Instruction::I64Load32U(AddressOffset::read_binary(sink)?),
            0x4C => Instruction::I32Store(AddressOffset::read_binary(sink)?),
            0x4D => Instruction::I64Store(AddressOffset::read_binary(sink)?),
            0x4E => Instruction::I32Store8(AddressOffset::read_binary(sink)?),
            0x4F => Instruction::I32Store16(AddressOffset::read_binary(sink)?),
            0x50 => Instruction::I64Store8(AddressOffset::read_binary(sink)?),
            0x51 => Instruction::I64Store16(AddressOffset::read_binary(sink)?),
            0x52 => Instruction::I64Store32(AddressOffset::read_binary(sink)?),

            // memory data Instruction family (?)
            0x53 => Instruction::MemorySize,
            0x54 => Instruction::MemoryGrow,
            0x55 => Instruction::MemoryFill,
            0x56 => Instruction::MemoryCopy,
            0x57 => Instruction::MemoryInit(DataSegmentIdx::read_binary(sink)?),
            0x58 => Instruction::DataDrop(DataSegmentIdx::read_binary(sink)?),
            0x59 => Instruction::TableSize(TableIdx::read_binary(sink)?),
            0x5A => Instruction::TableGrow(TableIdx::read_binary(sink)?),
            0x5B => Instruction::TableFill(TableIdx::read_binary(sink)?),
            0x5C => Instruction::TableGet(TableIdx::read_binary(sink)?),
            0x5D => Instruction::TableSet(TableIdx::read_binary(sink)?),
            0x5E => Instruction::TableCopy(TableIdx::read_binary(sink)?),
            0x5F => Instruction::TableInit(ElementSegmentIdx::read_binary(sink)?),
            // 0x60 => Instruction::ElemDrop(Index::read_binary(sink)?),
            // 0x61 => Instruction::RefFunc(Index::read_binary(sink)?),

            // i32/i64 Instruction family
            0x60 => Instruction::I64Const(UntypedValue::read_binary(sink)?),
            0x61 => Instruction::I32Const(UntypedValue::read_binary(sink)?),
            0x62 => Instruction::I32Eqz,
            0x63 => Instruction::I32Eq,
            0x64 => Instruction::I32Ne,
            0x65 => Instruction::I32LtS,
            0x66 => Instruction::I32LtU,
            0x67 => Instruction::I32GtS,
            0x68 => Instruction::I32GtU,
            0x69 => Instruction::I32LeS,
            0x6A => Instruction::I32LeU,
            0x6B => Instruction::I32GeS,
            0x6C => Instruction::I32GeU,
            0x6D => Instruction::I64Eqz,
            0x6E => Instruction::I64Eq,
            0x6F => Instruction::I64Ne,
            0x70 => Instruction::I64LtS,
            0x71 => Instruction::I64LtU,
            0x72 => Instruction::I64GtS,
            0x73 => Instruction::I64GtU,
            0x74 => Instruction::I64LeS,
            0x75 => Instruction::I64LeU,
            0x76 => Instruction::I64GeS,
            0x77 => Instruction::I64GeU,
            0x78 => Instruction::I32Clz,
            0x79 => Instruction::I32Ctz,
            0x7A => Instruction::I32Popcnt,
            0x7B => Instruction::I32Add,
            0x7C => Instruction::I32Sub,
            0x7D => Instruction::I32Mul,
            0x7E => Instruction::I32DivS,
            0x7F => Instruction::I32DivU,
            0x80 => Instruction::I32RemS,
            0x81 => Instruction::I32RemU,
            0x82 => Instruction::I32And,
            0x83 => Instruction::I32Or,
            0x84 => Instruction::I32Xor,
            0x85 => Instruction::I32Shl,
            0x86 => Instruction::I32ShrS,
            0x87 => Instruction::I32ShrU,
            0x88 => Instruction::I32Rotl,
            0x89 => Instruction::I32Rotr,
            0x8A => Instruction::I64Clz,
            0x8B => Instruction::I64Ctz,
            0x8C => Instruction::I64Popcnt,
            0x8D => Instruction::I64Add,
            0x8E => Instruction::I64Sub,
            0x8F => Instruction::I64Mul,
            0x90 => Instruction::I64DivS,
            0x91 => Instruction::I64DivU,
            0x92 => Instruction::I64RemS,
            0x93 => Instruction::I64RemU,
            0x94 => Instruction::I64And,
            0x95 => Instruction::I64Or,
            0x96 => Instruction::I64Xor,
            0x97 => Instruction::I64Shl,
            0x98 => Instruction::I64ShrS,
            0x99 => Instruction::I64ShrU,
            0x9A => Instruction::I64Rotl,
            0x9B => Instruction::I64Rotr,
            0x9C => Instruction::I32WrapI64,
            0x9D => Instruction::I64ExtendI32S,
            0x9E => Instruction::I64ExtendI32U,
            0x9F => Instruction::I32Extend8S,
            0xA0 => Instruction::I32Extend16S,
            0xA1 => Instruction::I64Extend8S,
            0xA2 => Instruction::I64Extend16S,
            0xA3 => Instruction::I64Extend32S,

            _ => return Err(BinaryFormatError::IllegalOpcode(byte)),
        })
    }
}
