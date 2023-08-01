use crate::{
    rwasm::binary_format::reader_writer::{BinaryFormatReader, BinaryFormatWriter},
    rwasm::binary_format::{BinaryFormat, BinaryFormatError},
};

impl<'a> BinaryFormat<'a> for u32 {
    type SelfType = u32;

    fn write_binary(&self, sink: &mut BinaryFormatWriter<'a>) -> Result<usize, BinaryFormatError> {
        sink.write_u32_be(*self)
    }

    fn read_binary(sink: &mut BinaryFormatReader<'a>) -> Result<u32, BinaryFormatError> {
        sink.read_u32_be()
    }
}

impl<'a> BinaryFormat<'a> for i32 {
    type SelfType = i32;

    fn write_binary(&self, sink: &mut BinaryFormatWriter<'a>) -> Result<usize, BinaryFormatError> {
        sink.write_i32_be(*self)
    }

    fn read_binary(sink: &mut BinaryFormatReader<'a>) -> Result<i32, BinaryFormatError> {
        sink.read_i32_be()
    }
}

impl<'a> BinaryFormat<'a> for u64 {
    type SelfType = u64;

    fn write_binary(&self, sink: &mut BinaryFormatWriter<'a>) -> Result<usize, BinaryFormatError> {
        sink.write_u64_be(*self)
    }

    fn read_binary(sink: &mut BinaryFormatReader<'a>) -> Result<u64, BinaryFormatError> {
        sink.read_u64_be()
    }
}

impl<'a> BinaryFormat<'a> for i64 {
    type SelfType = i64;

    fn write_binary(&self, sink: &mut BinaryFormatWriter<'a>) -> Result<usize, BinaryFormatError> {
        sink.write_i64_be(*self)
    }

    fn read_binary(sink: &mut BinaryFormatReader<'a>) -> Result<i64, BinaryFormatError> {
        sink.read_i64_be()
    }
}
