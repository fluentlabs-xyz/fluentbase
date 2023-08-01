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
