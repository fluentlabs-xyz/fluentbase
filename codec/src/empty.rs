use crate::{BufferDecoder, Encoder, WritableBuffer};

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct EmptyVec;

impl Encoder<EmptyVec> for EmptyVec {
    const HEADER_SIZE: usize = 12;

    fn encode<W: WritableBuffer>(&self, encoder: &mut W, field_offset: usize) {
        // first 4 bytes are number of elements
        encoder.write_u32(field_offset, 0);
        // remaining 4+4 are offset and length
        encoder.write_bytes(field_offset + 4, &[]);
    }

    fn decode_header(
        decoder: &mut BufferDecoder,
        field_offset: usize,
        _result: &mut EmptyVec,
    ) -> (usize, usize) {
        let count = decoder.read_u32(field_offset);
        debug_assert_eq!(count, 0);
        decoder.read_bytes_header(field_offset + 4)
    }
}
