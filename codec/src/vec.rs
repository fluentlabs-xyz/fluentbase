use crate::{BufferDecoder, BufferEncoder, Encoder};

///
/// We encode dynamic arrays as following:
/// - header
/// - + length - number of elements inside vector
/// - + offset - offset inside structure
/// - + size - number of encoded bytes
/// - body
/// - + raw bytes of the vector
impl<T: Default + Sized + Encoder<T>> Encoder<Vec<T>> for Vec<T> {
    fn header_size() -> usize {
        // u32: length + values (bytes)
        core::mem::size_of::<u32>() * 3
    }

    fn encode(&self, encoder: &mut BufferEncoder, field_offset: usize) {
        encoder.write_u32(field_offset, self.len() as u32);
        let mut value_encoder = BufferEncoder::new(T::header_size() * self.len(), None);
        for (i, obj) in self.iter().enumerate() {
            obj.encode(&mut value_encoder, T::header_size() * i);
        }
        encoder.write_bytes(field_offset + 4, value_encoder.finalize().as_slice());
    }

    fn decode(decoder: &mut BufferDecoder, field_offset: usize, result: &mut Vec<T>) {
        let input_len = decoder.read_u32(field_offset) as usize;
        let input_bytes = decoder.read_bytes(field_offset + 4);
        let mut value_decoder = BufferDecoder::new(input_bytes);
        *result = (0..input_len)
            .map(|i| {
                let mut result = T::default();
                T::decode(&mut value_decoder, T::header_size() * i, &mut result);
                result
            })
            .collect()
    }
}
