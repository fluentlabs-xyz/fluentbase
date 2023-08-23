mod instruction;
mod instruction_set;
mod number;
mod reader_writer;
mod utils;

pub use crate::rwasm::binary_format::reader_writer::{BinaryFormatReader, BinaryFormatWriter};
use alloc::vec::Vec;

#[derive(Debug, Copy, Clone)]
pub enum BinaryFormatError {
    ReachedUnreachable,
    NeedMore(usize),
    IllegalOpcode(u8),
}

pub trait BinaryFormat<'a> {
    type SelfType;

    fn write_binary_to_vec(&self, target: &'a mut Vec<u8>) -> Result<usize, BinaryFormatError> {
        let buf =
            unsafe { alloc::slice::from_raw_parts_mut(target.as_mut_ptr(), target.capacity()) };
        let mut sink = BinaryFormatWriter::<'a>::new(buf);
        let n = self.write_binary(&mut sink)?;
        target.resize(n, 0);
        sink.reset();
        let n = self.write_binary(&mut sink)?;
        Ok(n)
    }

    fn read_from_slice(sink: &'a [u8]) -> Result<Self::SelfType, BinaryFormatError> {
        let mut binary_format_reader = BinaryFormatReader::<'a>::new(sink);
        Self::read_binary(&mut binary_format_reader)
    }

    fn write_binary(&self, sink: &mut BinaryFormatWriter<'a>) -> Result<usize, BinaryFormatError>;

    fn read_binary(sink: &mut BinaryFormatReader<'a>) -> Result<Self::SelfType, BinaryFormatError>;
}
