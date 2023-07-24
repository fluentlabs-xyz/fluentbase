use crate::engine::bytecode::Instruction;
use crate::rwasm::binary_format::reader_writer::{BinaryFormatReader, BinaryFormatWriter};
use crate::rwasm::binary_format::{BinaryFormat, BinaryFormatError};
use crate::rwasm::instruction_set::InstructionSet;

impl<'a> BinaryFormat<'a> for InstructionSet {
    type SelfType = InstructionSet;

    fn write_binary(&self, sink: &mut BinaryFormatWriter<'a>) -> Result<(), BinaryFormatError> {
        for opcode in self.0.iter() {
            opcode.write_binary(sink)?;
        }
        Ok(())
    }

    fn read_binary(sink: &mut BinaryFormatReader<'a>) -> Result<InstructionSet, BinaryFormatError> {
        let mut result = InstructionSet::new();
        while !sink.is_empty() {
            result.push(Instruction::read_binary(sink)?);
        }
        Ok(result)
    }
}
