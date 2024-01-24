use crate::{buffer::WritableBuffer, BufferDecoder, Encoder};

impl Encoder<u8> for u8 {
    const HEADER_SIZE: usize = core::mem::size_of::<u8>();
    fn encode<W: WritableBuffer>(&self, encoder: &mut W, field_offset: usize) {
        encoder.write_u8(field_offset, *self);
    }
    fn decode_header(
        decoder: &mut BufferDecoder,
        field_offset: usize,
        result: &mut u8,
    ) -> (usize, usize) {
        *result = decoder.read_u8(field_offset);
        (0, 0)
    }
}

macro_rules! impl_le_int {
    ($typ:ty, $write_fn:ident, $read_fn:ident) => {
        impl Encoder<$typ> for $typ {
            const HEADER_SIZE: usize = core::mem::size_of::<$typ>();
            fn encode<W: WritableBuffer>(&self, encoder: &mut W, field_offset: usize) {
                encoder.$write_fn(field_offset, *self);
            }
            fn decode_header(
                decoder: &mut BufferDecoder,
                field_offset: usize,
                result: &mut $typ,
            ) -> (usize, usize) {
                *result = decoder.$read_fn(field_offset);
                (0, 0)
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

    fn encode<W: WritableBuffer>(&self, encoder: &mut W, field_offset: usize) {
        (0..N).for_each(|i| {
            self[i].encode(encoder, field_offset + i * T::HEADER_SIZE);
        });
    }

    fn decode_header(
        decoder: &mut BufferDecoder,
        field_offset: usize,
        result: &mut [T; N],
    ) -> (usize, usize) {
        (0..N).for_each(|i| {
            T::decode_body(decoder, field_offset + i * T::HEADER_SIZE, &mut result[i]);
        });
        (0, 0)
    }
}

impl<T: Sized + Encoder<T> + Default> Encoder<Option<T>> for Option<T> {
    const HEADER_SIZE: usize = 1 + T::HEADER_SIZE;

    fn encode<W: WritableBuffer>(&self, encoder: &mut W, field_offset: usize) {
        let option_flag = if self.is_some() { 1u8 } else { 0u8 };
        option_flag.encode(encoder, field_offset);
        if let Some(value) = &self {
            value.encode(encoder, field_offset + 1);
        } else {
            T::default().encode(encoder, field_offset + 1);
        }
    }

    fn decode_header(
        decoder: &mut BufferDecoder,
        field_offset: usize,
        result: &mut Option<T>,
    ) -> (usize, usize) {
        let mut option_flag: u8 = 0;
        u8::decode_header(decoder, field_offset, &mut option_flag);
        *result = if option_flag != 0 {
            let mut result_inner: T = Default::default();
            T::decode_header(decoder, field_offset + 1, &mut result_inner);
            Some(result_inner)
        } else {
            None
        };
        (0, 0)
    }

    fn decode_body(decoder: &mut BufferDecoder, field_offset: usize, result: &mut Option<T>) {
        let mut option_flag: u8 = 0;
        u8::decode_header(decoder, field_offset, &mut option_flag);
        *result = if option_flag != 0 {
            let mut result_inner: T = Default::default();
            T::decode_body(decoder, field_offset + 1, &mut result_inner);
            Some(result_inner)
        } else {
            None
        };
    }
}
