use crate::buffer::{BufferDecoder, BufferEncoder, FixedEncoder, WritableBuffer};
use alloc::vec::Vec;
use byteorder::ByteOrder;
use phantom_type::PhantomType;

pub const ALIGNMENT_DEFAULT: usize = 0; // 4 byte header items, not alignment for fields
pub const ALIGNMENT_32: usize = 32; // 4 byte header items, not alignment for fields
pub const HEADER_ITEM_SIZE_DEFAULT: usize = 4;

#[macro_export]
macro_rules! header_item_size {
    ($alignment:expr) => {
        if $alignment == $crate::encoder::ALIGNMENT_DEFAULT {
            $crate::encoder::HEADER_ITEM_SIZE_DEFAULT
        } else {
            $alignment
        }
    };
    ($alignment:expr, $typ:ty) => {
        if $alignment == $crate::encoder::ALIGNMENT_DEFAULT {
            core::mem::size_of::<$typ>()
        } else {
            $alignment
        }
    };
}

#[macro_export]
macro_rules! header_size {
    ($typ:ty, $endianness:ty, $alignment:expr) => {
        <$typ as Encoder<$endianness, $alignment, $typ>>::HEADER_SIZE
    };
}

#[macro_export]
macro_rules! call_encode {
    ($typ:ty, $endianness:ty, $alignment:expr, &$self:ident, &mut $encoder:ident, $field_offset:expr) => {
        <$typ as Encoder<$endianness, $alignment, $typ>>::encode(
            &$self,
            &mut $encoder,
            $field_offset,
        );
    };
}

#[macro_export]
macro_rules! call_decode_body {
    ($typ:ty, $endianness:ty, $alignment:expr, &mut $decoder:ident, $field_offset:expr, &mut $result:ident) => {
        <$typ as Encoder<$endianness, $alignment, $typ>>::decode_body(
            &mut $decoder,
            $field_offset,
            &mut $result,
        );
    };
}

#[macro_export]
macro_rules! align_number {
    ($number:expr, $alignment:expr) => {
        ($number + $alignment - 1) / $alignment * $alignment
    };
}

pub trait Encoder<E: ByteOrder, const A: usize, T: Sized> {
    const HEADER_SIZE: usize;

    fn header_size(&self) -> usize {
        Self::HEADER_SIZE
    }

    fn encode_to_fixed<const N: usize>(&self, field_offset: usize) -> ([u8; N], usize) {
        let mut buffer_encoder = FixedEncoder::<E, N>::new(Self::HEADER_SIZE);
        self.encode(&mut buffer_encoder, field_offset);
        buffer_encoder.finalize()
    }
    fn encode_to_vec(&self, field_offset: usize) -> Vec<u8> {
        let mut buffer_encoder = BufferEncoder::<E, A>::new(Self::HEADER_SIZE, None);
        self.encode(&mut buffer_encoder, field_offset);
        buffer_encoder.finalize()
    }

    fn encode<W: WritableBuffer<E>>(&self, encoder: &mut W, field_offset: usize);

    fn decode_header(
        decoder: &mut BufferDecoder<E>,
        field_offset: usize,
        result: &mut T,
    ) -> (usize, usize);

    fn decode_body(decoder: &mut BufferDecoder<E>, field_offset: usize, result: &mut T) {
        Self::decode_header(decoder, field_offset, result);
    }
}

pub struct FieldEncoder<
    E: ByteOrder,
    const A: usize,
    T: Sized + Encoder<E, A, T>,
    const FIELD_OFFSET: usize,
>(PhantomType<E>, PhantomType<T>);

impl<E: ByteOrder, const A: usize, T: Sized + Encoder<E, A, T>, const FIELD_OFFSET: usize>
    FieldEncoder<E, A, T, FIELD_OFFSET>
{
    pub const FIELD_OFFSET: usize = FIELD_OFFSET;
    pub const FIELD_SIZE: usize = T::HEADER_SIZE;

    pub fn decode_field_header(buffer: &[u8], result: &mut T) -> (usize, usize) {
        Self::decode_field_header_at(buffer, Self::FIELD_OFFSET, result)
    }

    pub fn decode_field_header_at(
        buffer: &[u8],
        field_offset: usize,
        result: &mut T,
    ) -> (usize, usize) {
        let mut buffer_decoder = BufferDecoder::new(buffer);
        T::decode_header(&mut buffer_decoder, field_offset, result)
    }

    pub fn decode_field_body(buffer: &[u8], result: &mut T) {
        Self::decode_field_body_at(buffer, Self::FIELD_OFFSET, result)
    }

    pub fn decode_field_body_at(buffer: &[u8], field_offset: usize, result: &mut T) {
        let mut buffer_decoder = BufferDecoder::new(buffer);
        T::decode_body(&mut buffer_decoder, field_offset, result)
    }
}
