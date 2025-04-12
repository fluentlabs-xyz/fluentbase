use crate::{
    binary_format::{BinaryFormat, BinaryFormatError, BinaryFormatReader},
    module::{Instruction, InstructionData},
};
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

pub fn decode_rwasm_instruction(
    sink: &mut BinaryFormatReader,
) -> Result<(Instruction, InstructionData), BinaryFormatError> {
    use Instruction::*;
    let byte = sink.read_u8()?;
    let instruction =
        Instruction::try_from(byte).map_err(|_| BinaryFormatError::IllegalOpcode(byte))?;
    let data = match instruction {
        LocalGet | LocalSet | LocalTee => {
            InstructionData::LocalDepth(LocalDepth::read_binary(sink)?)
        }
        Br | BrIfEqz | BrIfNez | BrAdjust | BrAdjustIfNez => {
            InstructionData::BranchOffset(BranchOffset::read_binary(sink)?)
        }
        BrTable => InstructionData::BranchTableTargets(BranchTableTargets::read_binary(sink)?),
        ConsumeFuel => InstructionData::BlockFuel(BlockFuel::read_binary(sink)?),
        Return | ReturnIfNez => InstructionData::DropKeep(DropKeep::read_binary(sink)?),
        ReturnCallInternal | CallInternal => {
            InstructionData::CompiledFunc(CompiledFunc::read_binary(sink)?)
        }
        ReturnCall | Call | RefFunc => InstructionData::FuncIdx(FuncIdx::read_binary(sink)?),
        ReturnCallIndirect | CallIndirect | SignatureCheck => {
            InstructionData::SignatureIdx(SignatureIdx::read_binary(sink)?)
        }
        GlobalGet | GlobalSet => InstructionData::GlobalIdx(GlobalIdx::read_binary(sink)?),
        I32Load | I64Load | F32Load | F64Load | I32Load8S | I32Load8U | I32Load16S | I32Load16U
        | I64Load8S | I64Load8U | I64Load16S | I64Load16U | I64Load32S | I64Load32U | I32Store
        | I64Store | F32Store | F64Store | I32Store8 | I32Store16 | I64Store8 | I64Store16
        | I64Store32 => InstructionData::AddressOffset(AddressOffset::read_binary(sink)?),
        MemoryInit | DataDrop => {
            InstructionData::DataSegmentIdx(DataSegmentIdx::read_binary(sink)?)
        }
        TableSize | TableGrow | TableFill | TableGet | TableSet | TableCopy => {
            InstructionData::TableIdx(TableIdx::read_binary(sink)?)
        }
        TableInit | ElemDrop => {
            InstructionData::ElementSegmentIdx(ElementSegmentIdx::read_binary(sink)?)
        }
        I32Const | I64Const | F32Const | F64Const => {
            InstructionData::UntypedValue(UntypedValue::read_binary(sink)?)
        }
        _ => InstructionData::EmptyData,
    };
    Ok((instruction, data))
}
