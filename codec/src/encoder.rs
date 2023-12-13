use crate::buffer::{BufferDecoder, BufferEncoder};
use core::hash::Hash;
use hashbrown::HashMap;

pub trait Encoder<T: Default + Sized> {
    fn header_size() -> usize;

    fn encode(&self, encoder: &mut BufferEncoder, field_offset: usize);

    fn decode(decoder: &mut BufferDecoder, field_offset: usize) -> T;
}

impl Encoder<u8> for u8 {
    fn header_size() -> usize {
        1
    }
    fn encode(&self, encoder: &mut BufferEncoder, field_offset: usize) {
        encoder.write_u8(field_offset, *self);
    }
    fn decode(decoder: &mut BufferDecoder, field_offset: usize) -> u8 {
        decoder.read_u8(field_offset)
    }
}

macro_rules! impl_le_int {
    ($typ:ty, $write_fn:ident, $read_fn:ident) => {
        impl Encoder<$typ> for $typ {
            fn header_size() -> usize {
                core::mem::size_of::<$typ>()
            }
            fn encode(&self, encoder: &mut BufferEncoder, field_offset: usize) {
                encoder.$write_fn(field_offset, *self);
            }
            fn decode(decoder: &mut BufferDecoder, field_offset: usize) -> $typ {
                decoder.$read_fn(field_offset)
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

// impl<T: Default + Sized + Encoder<T>, const N: usize> Encoder<[T; N]> for [T; N] {
//     fn header_size() -> usize {
//         T::header_size();
//         todo!()
//     }
//
//     fn encode(&self, encoder: &mut BufferEncoder, field_offset: usize) {
//         todo!()
//     }
//
//     fn decode(decoder: &mut BufferDecoder, field_offset: usize) -> [T; N] {
//         todo!()
//     }
// }

impl<T: Default + Sized + Encoder<T>> Encoder<Vec<T>> for Vec<T> {
    fn header_size() -> usize {
        // length + values (bytes)
        4 + 8
    }

    fn encode(&self, encoder: &mut BufferEncoder, field_offset: usize) {
        encoder.write_u32(field_offset, self.len() as u32);
        let mut value_encoder = BufferEncoder::new(T::header_size() * self.len(), None);
        for (i, obj) in self.iter().enumerate() {
            obj.encode(&mut value_encoder, T::header_size() * i);
        }
        encoder.write_bytes(field_offset + 4, value_encoder.finalize().as_slice());
    }

    fn decode(decoder: &mut BufferDecoder, field_offset: usize) -> Vec<T> {
        let input_len = decoder.read_u32(field_offset) as usize;
        let input_bytes = decoder.read_bytes(field_offset + 4);
        let mut value_decoder = BufferDecoder::new(input_bytes);
        (0..input_len)
            .map(|i| T::decode(&mut value_decoder, T::header_size() * i))
            .collect()
    }
}

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

    fn decode(decoder: &mut BufferDecoder, field_offset: usize) -> HashMap<K, V> {
        // decode length, keys and values
        let length = decoder.read_u32(field_offset) as usize;
        let (key_bytes, value_bytes) = decoder.read_bytes2(field_offset + 4, field_offset + 12);
        // decode keys
        let mut key_decoder = BufferDecoder::new(key_bytes);
        let keys = (0..length).map(|i| K::decode(&mut key_decoder, K::header_size() * i));
        // decode values
        let mut value_decoder = BufferDecoder::new(value_bytes);
        let values = (0..length).map(|i| V::decode(&mut value_decoder, V::header_size() * i));
        // zip into map
        keys.zip(values).collect()
    }
}

#[cfg(test)]
mod test {
    use super::Encoder;
    use crate::{BufferDecoder, BufferEncoder};
    use hashbrown::HashMap;

    #[test]
    fn test_vec() {
        let values = vec![0, 1, 2, 3];
        let result = {
            let mut buffer_encoder = BufferEncoder::new(12, None);
            values.encode(&mut buffer_encoder, 0);
            buffer_encoder.finalize()
        };
        println!("{}", hex::encode(&result));
        let mut buffer_decoder = BufferDecoder::new(result.as_slice());
        let values2 = Vec::<i32>::decode(&mut buffer_decoder, 0);
        assert_eq!(values, values2);
    }

    #[test]
    fn test_nested_vec() {
        let values = vec![vec![0, 1, 2], vec![3, 4, 5], vec![6, 7, 8]];
        let result = {
            let mut buffer_encoder = BufferEncoder::new(12, None);
            values.encode(&mut buffer_encoder, 0);
            buffer_encoder.finalize()
        };
        println!("{}", hex::encode(&result));
        let mut buffer_decoder = BufferDecoder::new(result.as_slice());
        let values2 = Vec::<Vec<i32>>::decode(&mut buffer_decoder, 0);
        assert_eq!(values, values2);
    }

    #[test]
    fn test_empty_vec() {
        let values: Vec<u32> = vec![];
        let result = {
            let mut buffer_encoder = BufferEncoder::new(12, None);
            values.encode(&mut buffer_encoder, 0);
            buffer_encoder.finalize()
        };
        println!("{}", hex::encode(&result));
        let mut buffer_decoder = BufferDecoder::new(result.as_slice());
        let values2 = Vec::<u32>::decode(&mut buffer_decoder, 0);
        assert_eq!(values, values2);
    }

    #[test]
    fn test_map() {
        let mut values = HashMap::new();
        values.insert(100, 20);
        values.insert(3, 5);
        values.insert(1000, 60);
        let result = {
            let mut buffer_encoder = BufferEncoder::new(20, None);
            values.encode(&mut buffer_encoder, 0);
            buffer_encoder.finalize()
        };
        println!("{}", hex::encode(&result));
        let mut buffer_decoder = BufferDecoder::new(result.as_slice());
        let values2 = HashMap::decode(&mut buffer_decoder, 0);
        assert_eq!(values, values2);
    }

    #[test]
    fn test_nested_map() {
        let mut values = HashMap::new();
        values.insert(100, HashMap::from([(1, 2), (3, 4)]));
        values.insert(3, HashMap::new());
        values.insert(1000, HashMap::from([(7, 8), (9, 4)]));
        let result = {
            let mut buffer_encoder = BufferEncoder::new(20, None);
            values.encode(&mut buffer_encoder, 0);
            buffer_encoder.finalize()
        };
        println!("{}", hex::encode(&result));
        let mut buffer_decoder = BufferDecoder::new(result.as_slice());
        let values2 = HashMap::decode(&mut buffer_decoder, 0);
        assert_eq!(values, values2);
    }

    #[test]
    fn test_vector_of_maps() {
        let mut values = Vec::new();
        values.push(HashMap::from([(1, 2), (3, 4)]));
        values.push(HashMap::new());
        values.push(HashMap::from([(7, 8), (9, 4)]));
        let result = {
            let mut buffer_encoder = BufferEncoder::new(20, None);
            values.encode(&mut buffer_encoder, 0);
            buffer_encoder.finalize()
        };
        println!("{}", hex::encode(&result));
        let mut buffer_decoder = BufferDecoder::new(result.as_slice());
        let values2 = Vec::decode(&mut buffer_decoder, 0);
        assert_eq!(values, values2);
    }

    #[test]
    fn test_map_of_vectors() {
        let mut values = HashMap::new();
        values.insert(vec![0, 1, 2], vec![3, 4, 5]);
        values.insert(vec![3, 1, 2], vec![3, 4, 5]);
        values.insert(vec![0, 1, 6], vec![3, 4, 5]);
        let result = {
            let mut buffer_encoder = BufferEncoder::new(20, None);
            values.encode(&mut buffer_encoder, 0);
            buffer_encoder.finalize()
        };
        println!("{}", hex::encode(&result));
        let mut buffer_decoder = BufferDecoder::new(result.as_slice());
        let values2 = HashMap::decode(&mut buffer_decoder, 0);
        assert_eq!(values, values2);
    }
}
