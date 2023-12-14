use crate::buffer::{BufferDecoder, BufferEncoder};

pub trait Encoder<T: Sized> {
    fn header_size() -> usize;

    fn encode(&self, encoder: &mut BufferEncoder, field_offset: usize);

    fn decode(decoder: &mut BufferDecoder, field_offset: usize, result: &mut T);
}
