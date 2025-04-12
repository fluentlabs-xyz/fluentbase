use crate::binary_format::{
    reader_writer::{BinaryFormatReader, BinaryFormatWriter},
    BinaryFormat,
    BinaryFormatError,
};

macro_rules! impl_primitive_format {
    ($ty:ty, $write_method:ident, $read_method:ident) => {
        impl<'a> BinaryFormat<'a> for $ty {
            type SelfType = $ty;
            fn encoded_length(&self) -> usize {
                core::mem::size_of::<$ty>()
            }
            fn write_binary(
                &self,
                sink: &mut BinaryFormatWriter<'a>,
            ) -> Result<usize, BinaryFormatError> {
                sink.$write_method(*self)
            }
            fn read_binary(sink: &mut BinaryFormatReader<'a>) -> Result<$ty, BinaryFormatError> {
                sink.$read_method()
            }
        }
    };
}

impl_primitive_format!(u16, write_u16_le, read_u16_le);
impl_primitive_format!(i16, write_i16_le, read_i16_le);
impl_primitive_format!(u32, write_u32_le, read_u32_le);
impl_primitive_format!(i32, write_i32_le, read_i32_le);
impl_primitive_format!(u64, write_u64_le, read_u64_le);
impl_primitive_format!(i64, write_i64_le, read_i64_le);
