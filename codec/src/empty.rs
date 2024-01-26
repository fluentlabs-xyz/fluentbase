use crate::{BufferDecoder, Encoder, WritableBuffer};

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct EmptyVec;

impl Encoder<EmptyVec> for EmptyVec {
    const HEADER_SIZE: usize = 8;

    fn encode<W: WritableBuffer>(&self, encoder: &mut W, field_offset: usize) {
        encoder.write_bytes(field_offset, &[]);
    }

    fn decode_header(
        decoder: &mut BufferDecoder,
        field_offset: usize,
        _result: &mut EmptyVec,
    ) -> (usize, usize) {
        decoder.read_bytes_header(field_offset)
    }
}
