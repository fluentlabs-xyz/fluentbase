mod drop_keep;
pub mod instruction;
pub mod module;
mod number;
pub mod reader_writer;
mod utils;

pub use self::reader_writer::{BinaryFormatReader, BinaryFormatWriter};
use alloc::vec::Vec;

#[derive(Debug, Copy, Clone)]
pub enum BinaryFormatError {
    NeedMore(usize),
    MalformedWasmModule,
    IllegalOpcode(u8),
}

pub trait BinaryFormat<'a> {
    type SelfType;

    fn encoded_length(&self) -> usize;

    fn write_binary_to_vec(&self, buffer: &'a mut Vec<u8>) -> Result<usize, BinaryFormatError> {
        buffer.resize(self.encoded_length(), 0u8);
        let mut sink = BinaryFormatWriter::<'a>::new(buffer.as_mut_slice());
        self.write_binary(&mut sink)
    }

    fn write_binary(&self, sink: &mut BinaryFormatWriter<'a>) -> Result<usize, BinaryFormatError>;

    fn read_from_slice(sink: &'a [u8]) -> Result<Self::SelfType, BinaryFormatError> {
        let mut binary_format_reader = BinaryFormatReader::<'a>::new(sink);
        Self::read_binary(&mut binary_format_reader)
    }

    fn read_binary(sink: &mut BinaryFormatReader<'a>) -> Result<Self::SelfType, BinaryFormatError>;
}
