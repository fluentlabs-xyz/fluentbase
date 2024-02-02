use crate::buffer::{BufferDecoder, BufferEncoder, FixedEncoder, WritableBuffer};
use alloc::vec::Vec;
use core::marker::PhantomData;

pub trait Encoder<T: Sized> {
    fn header_size(&self) -> usize {
        Self::HEADER_SIZE
    }

    const HEADER_SIZE: usize;

    fn encode_to_fixed<const N: usize>(&self, field_offset: usize) -> ([u8; N], usize) {
        let mut buffer_encoder = FixedEncoder::<N>::new(Self::HEADER_SIZE);
        self.encode(&mut buffer_encoder, field_offset);
        buffer_encoder.finalize()
    }
    fn encode_to_vec(&self, field_offset: usize) -> Vec<u8> {
        let mut buffer_encoder = BufferEncoder::new(Self::HEADER_SIZE, None);
        self.encode(&mut buffer_encoder, field_offset);
        buffer_encoder.finalize()
    }

    fn encode<W: WritableBuffer>(&self, encoder: &mut W, field_offset: usize);

    fn decode_header(
        decoder: &mut BufferDecoder,
        field_offset: usize,
        result: &mut T,
    ) -> (usize, usize);

    fn decode_body(decoder: &mut BufferDecoder, field_offset: usize, result: &mut T) {
        Self::decode_header(decoder, field_offset, result);
    }
}

pub struct FieldEncoder<T: Sized + Encoder<T>, const FIELD_OFFSET: usize>(PhantomData<T>);

impl<T: Sized + Encoder<T>, const FIELD_OFFSET: usize> FieldEncoder<T, FIELD_OFFSET> {
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
        <T as Encoder<T>>::decode_header(&mut buffer_decoder, field_offset, result)
    }

    pub fn decode_field_body(buffer: &[u8], result: &mut T) {
        Self::decode_field_body_at(buffer, Self::FIELD_OFFSET, result)
    }

    pub fn decode_field_body_at(buffer: &[u8], field_offset: usize, result: &mut T) {
        let mut buffer_decoder = BufferDecoder::new(buffer);
        <T as Encoder<T>>::decode_body(&mut buffer_decoder, field_offset, result)
    }
}
