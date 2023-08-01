use crate::{
    engine::bytecode::Instruction,
    rwasm::binary_format::reader_writer::{BinaryFormatReader, BinaryFormatWriter},
    rwasm::binary_format::{BinaryFormat, BinaryFormatError},
    rwasm::instruction_set::InstructionSet,
};

impl<'a> BinaryFormat<'a> for InstructionSet {
    type SelfType = InstructionSet;

    fn write_binary(&self, sink: &mut BinaryFormatWriter<'a>) -> Result<usize, BinaryFormatError> {
        let mut n = 0;
        for opcode in self.0.iter() {
            n += opcode.write_binary(sink)?;
        }
        Ok(n)
    }

    fn read_binary(sink: &mut BinaryFormatReader<'a>) -> Result<InstructionSet, BinaryFormatError> {
        let mut result = InstructionSet::new();
        while !sink.is_empty() {
            result.push(Instruction::read_binary(sink)?);
        }
        Ok(result)
    }
}
