use crate::{BufferDecoder, BufferEncoder, Encoder};

impl Encoder<u8> for u8 {
    const HEADER_SIZE: usize = core::mem::size_of::<u8>();
    fn encode(&self, encoder: &mut BufferEncoder, field_offset: usize) {
        encoder.write_u8(field_offset, *self);
    }
    fn decode(decoder: &mut BufferDecoder, field_offset: usize, result: &mut u8) {
        *result = decoder.read_u8(field_offset)
    }
}

macro_rules! impl_le_int {
    ($typ:ty, $write_fn:ident, $read_fn:ident) => {
        impl Encoder<$typ> for $typ {
            const HEADER_SIZE: usize = core::mem::size_of::<$typ>();
            fn encode(&self, encoder: &mut BufferEncoder, field_offset: usize) {
                encoder.$write_fn(field_offset, *self);
            }
            fn decode(decoder: &mut BufferDecoder, field_offset: usize, result: &mut $typ) {
                *result = decoder.$read_fn(field_offset);
            }
        }
    };
}

impl_le_int!(u16, write_u16, read_u16);
impl_le_int!(u32, write_u32, read_u32);
impl_le_int!(u64, write_u64, read_u64);
impl_le_int!(i16, write_i16, read_i16);
impl_le_int!(i32, write_i32, read_i32);
impl_le_int!(i64, write_i64, read_i64);

impl<T: Sized + Encoder<T>, const N: usize> Encoder<[T; N]> for [T; N] {
    const HEADER_SIZE: usize = T::HEADER_SIZE * N;

    fn encode(&self, encoder: &mut BufferEncoder, field_offset: usize) {
        (0..N).for_each(|i| {
            self[i].encode(encoder, field_offset + i * T::HEADER_SIZE);
        });
    }

    fn decode(decoder: &mut BufferDecoder, field_offset: usize, result: &mut [T; N]) {
        (0..N).for_each(|i| {
            T::decode(decoder, field_offset + i * T::HEADER_SIZE, &mut result[i]);
        });
    }
}
