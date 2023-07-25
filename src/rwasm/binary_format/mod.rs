mod instruction;
mod instruction_set;
mod number;
mod reader_writer;
mod utils;

use alloc::vec::Vec;

pub use crate::rwasm::binary_format::reader_writer::{BinaryFormatReader, BinaryFormatWriter};

#[derive(Debug, Copy, Clone)]
pub enum BinaryFormatError {
    NeedMore(usize),
    IllegalOpcode(u8),
}

pub trait BinaryFormat<'a> {
    type SelfType;

    fn write_binary_to_vec(&self, target: &'a mut Vec<u8>) -> Result<usize, BinaryFormatError> {
        let buf = unsafe { alloc::slice::from_raw_parts_mut(target.as_mut_ptr(), target.capacity()) };
        let mut sink = BinaryFormatWriter::<'a>::new(buf);
        let n = self.write_binary(&mut sink)?;
        target.resize(n, 0);
        sink.reset();
        let n = self.write_binary(&mut sink)?;
        Ok(n)
    }

    fn write_binary(&self, sink: &mut BinaryFormatWriter<'a>) -> Result<usize, BinaryFormatError>;
    fn read_binary(sink: &mut BinaryFormatReader<'a>) -> Result<Self::SelfType, BinaryFormatError>;
}

#[cfg(test)]
mod tests {
    use crate::{
        engine::bytecode::Instruction,
        engine::CompiledFunc,
        rwasm::binary_format::reader_writer::{BinaryFormatReader, BinaryFormatWriter},
        rwasm::binary_format::BinaryFormat,
    };
    use alloc::vec::Vec;
    use strum::IntoEnumIterator;

    #[test]
    fn test_opcode_encoding() {
        for opcode in Instruction::iter() {
            let mut buf = vec![0; 100];
            let mut writer = BinaryFormatWriter::new(buf.as_mut_slice());
            opcode.write_binary(&mut writer).unwrap();
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
