use crate::{BufferDecoder, BufferEncoder, Encoder};
use hashbrown::{HashMap, HashSet};
use std::hash::Hash;

impl<K: Default + Sized + Encoder<K> + Eq + Hash, V: Default + Sized + Encoder<V>>
    Encoder<HashMap<K, V>> for HashMap<K, V>
{
    fn header_size() -> usize {
        // length + keys (bytes) + values (bytes)
        4 + 8 + 8
    }

    fn encode(&self, encoder: &mut BufferEncoder, field_offset: usize) {
        // encode length
        encoder.write_u32(field_offset, self.len() as u32);
        // encode keys
        let mut key_encoder = BufferEncoder::new(K::header_size() * self.len(), None);
        for (i, obj) in self.keys().enumerate() {
            obj.encode(&mut key_encoder, K::header_size() * i);
        }
        encoder.write_bytes(field_offset + 4, key_encoder.finalize().as_slice());
        // encode values
        let mut value_encoder = BufferEncoder::new(V::header_size() * self.len(), None);
        for (i, obj) in self.values().enumerate() {
            obj.encode(&mut value_encoder, V::header_size() * i);
        }
        encoder.write_bytes(field_offset + 12, value_encoder.finalize().as_slice());
    }

    fn decode(decoder: &mut BufferDecoder, field_offset: usize, result: &mut HashMap<K, V>) {
        // decode length, keys and values
        let length = decoder.read_u32(field_offset) as usize;
        let (key_bytes, value_bytes) = decoder.read_bytes2(field_offset + 4, field_offset + 12);
        // decode keys
        let mut key_decoder = BufferDecoder::new(key_bytes);
        let keys = (0..length).map(|i| {
            let mut result = Default::default();
            K::decode(&mut key_decoder, K::header_size() * i, &mut result);
            result
        });
        // decode values
        let mut value_decoder = BufferDecoder::new(value_bytes);
        let values = (0..length).map(|i| {
            let mut result = Default::default();
            V::decode(&mut value_decoder, V::header_size() * i, &mut result);
            result
        });
        // zip into map
        *result = keys.zip(values).collect()
    }
}

impl<T: Default + Sized + Encoder<T> + Eq + Hash> Encoder<HashSet<T>> for HashSet<T> {
    fn header_size() -> usize {
        // length + keys (bytes)
        4 + 8
    }

    fn encode(&self, encoder: &mut BufferEncoder, field_offset: usize) {
        // encode length
        encoder.write_u32(field_offset, self.len() as u32);
        // encode values
        let mut value_encoder = BufferEncoder::new(T::header_size() * self.len(), None);
        for (i, obj) in self.iter().enumerate() {
            obj.encode(&mut value_encoder, T::header_size() * i);
        }
        encoder.write_bytes(field_offset + 4, value_encoder.finalize().as_slice());
    }

    fn decode(decoder: &mut BufferDecoder, field_offset: usize, result: &mut HashSet<T>) {
        // decode length, keys and values
        let length = decoder.read_u32(field_offset) as usize;
        let value_bytes = decoder.read_bytes(field_offset + 4);
        // decode values
        let mut value_decoder = BufferDecoder::new(value_bytes);
        let values = (0..length).map(|i| {
            let mut result = Default::default();
            T::decode(&mut value_decoder, T::header_size() * i, &mut result);
            result
        });
        // zip into map
        *result = values.collect()
    }
}
