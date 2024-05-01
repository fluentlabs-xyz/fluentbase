use crate::{BufferDecoder, Encoder, WritableBuffer};

impl<A1: Encoder<A1>, A2: Encoder<A2>> Encoder<(A1, A2)> for (A1, A2) {
    const HEADER_SIZE: usize = A1::HEADER_SIZE + A2::HEADER_SIZE;

    fn encode<W: WritableBuffer>(&self, encoder: &mut W, field_offset: usize) {
        self.0.encode(encoder, field_offset);
        self.1.encode(encoder, field_offset + A1::HEADER_SIZE);
    }

    fn decode_header(
        decoder: &mut BufferDecoder,
        field_offset: usize,
        result: &mut (A1, A2),
    ) -> (usize, usize) {
        let (offset1, _) = A1::decode_header(decoder, field_offset, &mut result.0);
        let (_, length2) =
            A2::decode_header(decoder, field_offset + A1::HEADER_SIZE, &mut result.1);
        (offset1, length2)
    }

    fn decode_body(decoder: &mut BufferDecoder, field_offset: usize, result: &mut (A1, A2)) {
        A1::decode_body(decoder, field_offset, &mut result.0);
        A2::decode_body(decoder, field_offset + A1::HEADER_SIZE, &mut result.1);
    }
}
