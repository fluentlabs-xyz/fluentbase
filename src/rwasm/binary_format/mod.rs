mod instruction;
mod instruction_set;
mod number;
mod reader_writer;
mod utils;

use crate::rwasm::binary_format::reader_writer::{BinaryFormatReader, BinaryFormatWriter};
use alloc::vec::Vec;
use byteorder::ByteOrder;

#[derive(Debug, Copy, Clone)]
pub enum BinaryFormatError {
    NeedMore(usize),
    IllegalOpcode(u8),
}

pub trait BinaryFormat<'a> {
    type SelfType;

    fn write_binary_to_vec(&self, buf: &'a mut Vec<u8>) -> Result<(), BinaryFormatError> {
        let buf = unsafe { alloc::slice::from_raw_parts_mut(buf.as_mut_ptr(), buf.capacity()) };
        let mut sink = BinaryFormatWriter::<'a>::new(buf);
        self.write_binary(&mut sink)?;
        Ok(())
    }

    fn write_binary(&self, sink: &mut BinaryFormatWriter<'a>) -> Result<(), BinaryFormatError>;
    fn read_binary(sink: &mut BinaryFormatReader<'a>) -> Result<Self::SelfType, BinaryFormatError>;
}

#[cfg(test)]
mod tests {
    use crate::engine::bytecode::Instruction;
    use crate::rwasm::binary_format::reader_writer::{BinaryFormatReader, BinaryFormatWriter};
    use crate::rwasm::binary_format::BinaryFormat;
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
}
