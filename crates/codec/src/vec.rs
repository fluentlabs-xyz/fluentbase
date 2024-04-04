use crate::{buffer::WritableBuffer, BufferDecoder, BufferEncoder, Encoder};
use alloc::vec::Vec;

///
/// We encode dynamic arrays as following:
/// - header
/// - + length - number of elements inside vector
/// - + offset - offset inside structure
/// - + size - number of encoded bytes
/// - body
/// - + raw bytes of the vector
///
/// We don't encode empty vectors, instead of store only 0 length,
/// it helps to reduce empty vector size from 12 to 4 bytes.
impl<T: Default + Sized + Encoder<T>> Encoder<Vec<T>> for Vec<T> {
    // u32: length + values (bytes)
    const HEADER_SIZE: usize = core::mem::size_of::<u32>() * 3;

    fn encode<W: WritableBuffer>(&self, encoder: &mut W, field_offset: usize) {
        encoder.write_u32(field_offset, self.len() as u32);
        let mut value_encoder = BufferEncoder::new(T::HEADER_SIZE * self.len(), None);
        for (i, obj) in self.iter().enumerate() {
            obj.encode(&mut value_encoder, T::HEADER_SIZE * i);
        }
        encoder.write_bytes(field_offset + 4, value_encoder.finalize().as_slice());
    }

    fn decode_header(
        decoder: &mut BufferDecoder,
        field_offset: usize,
        result: &mut Vec<T>,
    ) -> (usize, usize) {
        let count = decoder.read_u32(field_offset) as usize;
        if count > result.capacity() {
            result.reserve(count - result.capacity());
        }
        let (offset, length) = decoder.read_bytes_header(field_offset + 4);
        (offset, length)
    }

    fn decode_body(decoder: &mut BufferDecoder, field_offset: usize, result: &mut Vec<T>) {
        let input_len = decoder.read_u32(field_offset) as usize;
        if input_len == 0 {
            result.clear();
            return;
        }
        let input_bytes = decoder.read_bytes(field_offset + 4);
        let mut value_decoder = BufferDecoder::new(input_bytes);
        *result = (0..input_len)
            .map(|i| {
                let mut result = T::default();
                T::decode_body(&mut value_decoder, T::HEADER_SIZE * i, &mut result);
                result
            })
            .collect()
    }
}
