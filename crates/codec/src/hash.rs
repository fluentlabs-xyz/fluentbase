use crate::{
    bytes_codec::{read_bytes_header, write_bytes, write_bytes_solidity, write_bytes_wasm},
    encoder::{align_up, read_u32_aligned, write_u32_aligned, Encoder},
    error::{CodecError, DecodingError},
};
use alloc::{format, string::ToString, vec::Vec};
use byteorder::ByteOrder;
use bytes::{Buf, BufMut, BytesMut};
use core::{fmt::Debug, hash::Hash};
use hashbrown::{HashMap, HashSet};

/// Implement encoding for HashMap, SOL_MODE = false
impl<K, V, B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, false, false> for HashMap<K, V>
where
    K: Default + Sized + Encoder<B, ALIGN, false, false> + Eq + Hash + Ord,
    V: Default + Sized + Encoder<B, ALIGN, false, false>,
{
    const HEADER_SIZE: usize = 4 + 8 + 8; // length + keys_header + values_header
    const IS_DYNAMIC: bool = true;

    fn encode(&self, buf: &mut impl BufMut, mut offset: usize) -> Result<usize, CodecError> {
        let offset_before = offset;
        // Write map size
        offset += write_u32_aligned::<B, ALIGN>(buf, self.len() as u32);
        // Make sure keys and values are sorted
        let mut entries: Vec<_> = self.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        // Encode and write keys
        for (key, _) in entries.iter() {
            offset += key.encode(buf, offset)?;
        }
        // Encode and write values
        for (_, value) in entries.iter() {
            offset += value.encode(buf, offset)?;
        }
        Ok(offset - offset_before)
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let aligned_header_el_size = align_up::<ALIGN>(4);
        let aligned_header_size = align_up::<ALIGN>(Self::HEADER_SIZE);

        if buf.remaining() < offset + aligned_header_size {
            return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                expected: offset + aligned_header_size,
                found: buf.remaining(),
                msg: "Not enough data to decode HashMap header".to_string(),
            }));
        }

        let length = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;

        let (keys_offset, keys_length) =
            read_bytes_header::<B, ALIGN, false>(buf, offset + aligned_header_el_size)?;

        let (values_offset, values_length) =
            read_bytes_header::<B, ALIGN, false>(buf, offset + aligned_header_el_size * 3)?;

        let key_bytes = &buf.chunk()[keys_offset..keys_offset + keys_length];
        let value_bytes = &buf.chunk()[values_offset..values_offset + values_length];

        let keys = (0..length).map(|i| {
            let key_offset = align_up::<ALIGN>(K::HEADER_SIZE) * i;
            K::decode(&key_bytes, key_offset).unwrap_or_default()
        });

        let values = (0..length).map(|i| {
            let value_offset = align_up::<ALIGN>(V::HEADER_SIZE) * i;
            V::decode(&value_bytes, value_offset).unwrap_or_default()
        });

        let result: HashMap<K, V> = keys.zip(values).collect();

        if result.len() != length {
            return Err(CodecError::Decoding(DecodingError::InvalidData(format!(
                "Expected {} elements, but decoded {}",
                length,
                result.len()
            ))));
        }

        Ok(result)
    }

    fn partial_decode(buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        let aligned_header_size = align_up::<ALIGN>(Self::HEADER_SIZE);

        if buf.remaining() < offset + aligned_header_size {
            return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                expected: offset + aligned_header_size,
                found: buf.remaining(),
                msg: "Not enough data to decode HashMap header".to_string(),
            }));
        }

        let (keys_offset, keys_length) =
            read_bytes_header::<B, ALIGN, false>(buf, offset + align_up::<ALIGN>(4))?;
        let (_values_offset, values_length) =
            read_bytes_header::<B, ALIGN, false>(buf, offset + align_up::<ALIGN>(12))?;

        Ok((keys_offset, keys_length + values_length))
    }
}
/// Implement encoding for HashMap, SOL_MODE = true
impl<K, V, B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, true, false> for HashMap<K, V>
where
    K: Debug + Default + Sized + Encoder<B, ALIGN, true, false> + Eq + Hash + Ord,
    V: Debug + Default + Sized + Encoder<B, ALIGN, true, false>,
{
    const HEADER_SIZE: usize = 32 + 32 + 32 + 32; // offset + length + keys_header + values_header

    const IS_DYNAMIC: bool = true;

    fn encode(&self, buf: &mut impl BufMut, mut offset: usize) -> Result<usize, CodecError> {
        let offset_before = offset;
        // Write map size
        offset += write_u32_aligned::<B, ALIGN>(buf, self.len() as u32);
        // Make sure keys and values are sorted
        let mut entries: Vec<_> = self.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        // Encode and write keys
        for (key, _) in entries.iter() {
            offset += key.encode(buf, offset)?;
        }
        // Encode and write values
        for (_, value) in entries.iter() {
            offset += value.encode(buf, offset)?;
        }
        Ok(offset - offset_before)
    }

    // current solidity decode nested map
    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        const KEYS_OFFSET: usize = 32;
        const VALUES_OFFSET: usize = 64;

        // Check if there's enough data to read the header
        let header_end = offset
            .checked_add(Self::HEADER_SIZE)
            .ok_or(CodecError::Decoding(DecodingError::Overflow))?;

        if buf.remaining() < header_end {
            return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                expected: header_end,
                found: buf.remaining(),
                msg: "Not enough data to decode HashMap header".to_string(),
            }));
        }

        // Read data offset
        let data_offset = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;

        // Calculate start offset
        let start_offset = offset
            .checked_add(data_offset)
            .ok_or(CodecError::Decoding(DecodingError::Overflow))?;

        // Read length
        let length = read_u32_aligned::<B, ALIGN>(buf, start_offset)? as usize;
        if length == 0 {
            return Ok(HashMap::new());
        }

        // Read relative keys and values offsets (relative to the current offset)
        let keys_offset = read_u32_aligned::<B, ALIGN>(buf, start_offset + KEYS_OFFSET)? as usize;
        let values_offset =
            read_u32_aligned::<B, ALIGN>(buf, start_offset + VALUES_OFFSET)? as usize;

        // Calculate absolute offsets
        let keys_start = keys_offset
            .checked_add(start_offset)
            .and_then(|sum| sum.checked_add(KEYS_OFFSET))
            .ok_or(CodecError::Decoding(DecodingError::Overflow))?;
        let values_start = values_offset
            .checked_add(start_offset)
            .and_then(|sum| sum.checked_add(VALUES_OFFSET))
            .ok_or(CodecError::Decoding(DecodingError::Overflow))?;

        let mut result = HashMap::with_capacity(length);

        let keys_data = &buf.chunk()[keys_start + 32..];
        let values_data = &buf.chunk()[values_start + 32..];

        for i in 0..length {
            let key_offset = align_up::<ALIGN>(K::HEADER_SIZE)
                .checked_mul(i)
                .ok_or(CodecError::Decoding(DecodingError::Overflow))?;
            let value_offset = align_up::<ALIGN>(V::HEADER_SIZE)
                .checked_mul(i)
                .ok_or(CodecError::Decoding(DecodingError::Overflow))?;

            let key = K::decode(&keys_data, key_offset)?;
            let value = V::decode(&values_data, value_offset)?;

            result.insert(key, value);
        }

        Ok(result)
    }

    fn partial_decode(buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        let aligned_header_size = align_up::<ALIGN>(Self::HEADER_SIZE);

        if buf.remaining() < offset + aligned_header_size {
            return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                expected: offset + aligned_header_size,
                found: buf.remaining(),
                msg: "Not enough data to decode HashMap header".to_string(),
            }));
        }

        let (keys_offset, keys_length) =
            read_bytes_header::<B, ALIGN, false>(buf, offset + align_up::<ALIGN>(4))?;
        let (_values_offset, values_length) =
            read_bytes_header::<B, ALIGN, false>(buf, offset + align_up::<ALIGN>(12))?;

        Ok((keys_offset, keys_length + values_length))
    }
}

/// Implement encoding for HashSet, SOL_MODE = false
impl<T, B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, false, false> for HashSet<T>
where
    T: Default + Sized + Encoder<B, ALIGN, false, false> + Eq + Hash + Ord,
{
    const HEADER_SIZE: usize = 4 + 8; // length + data_header
    const IS_DYNAMIC: bool = true;

    fn encode(&self, buf: &mut impl BufMut, mut offset: usize) -> Result<usize, CodecError> {
        let offset_before = offset;
        // Write set size
        write_u32_aligned::<B, ALIGN>(buf, self.len() as u32);
        // Make sure a set is sorted
        let mut entries: Vec<_> = self.iter().collect();
        entries.sort();
        // Encode values
        for value in entries {
            offset += value.encode(buf, offset)?;
        }
        Ok(offset - offset_before)
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let aligned_offset = align_up::<ALIGN>(offset);
        let aligned_header_size = align_up::<ALIGN>(Self::HEADER_SIZE);

        if buf.remaining() < aligned_offset + aligned_header_size {
            return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                expected: aligned_offset + aligned_header_size,
                found: buf.remaining(),
                msg: "Not enough data to decode HashSet header".to_string(),
            }));
        }

        let length = read_u32_aligned::<B, ALIGN>(buf, aligned_offset)? as usize;

        let (data_offset, data_length) =
            read_bytes_header::<B, ALIGN, false>(buf, aligned_offset + align_up::<ALIGN>(4))?;

        let mut result = HashSet::with_capacity(length);

        let value_bytes = &buf.chunk()[data_offset..data_offset + data_length];

        for i in 0..length {
            let value_offset = align_up::<ALIGN>(T::HEADER_SIZE) * i;
            let value = T::decode(&value_bytes, value_offset)?;
            result.insert(value);
        }

        if result.len() != length {
            return Err(CodecError::Decoding(DecodingError::InvalidData(format!(
                "Expected {} elements, but decoded {}",
                length,
                result.len()
            ))));
        }

        Ok(result)
    }

    fn partial_decode(buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        let aligned_offset = align_up::<ALIGN>(offset);
        let aligned_header_size = align_up::<ALIGN>(Self::HEADER_SIZE);

        if buf.remaining() < aligned_offset + aligned_header_size {
            return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                expected: aligned_offset + aligned_header_size,
                found: buf.remaining(),
                msg: "Not enough data to decode HashSet header".to_string(),
            }));
        }

        let (data_offset, data_length) =
            read_bytes_header::<B, ALIGN, false>(buf, aligned_offset + align_up::<ALIGN>(4))?;

        Ok((data_offset, data_length))
    }
}

/// Implement encoding for HashSet, SOL_MODE = true
impl<T, B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, true, false> for HashSet<T>
where
    T: Debug + Default + Sized + Encoder<B, ALIGN, true, false> + Eq + Hash + Ord,
{
    const HEADER_SIZE: usize = 32 + 32 + 32; // offset + length + data_header
    const IS_DYNAMIC: bool = true;

    fn encode(&self, buf: &mut impl BufMut, mut offset: usize) -> Result<usize, CodecError> {
        let offset_before = offset;
        // Write set size
        write_u32_aligned::<B, ALIGN>(buf, self.len() as u32);
        // Make sure a set is sorted
        let mut entries: Vec<_> = self.iter().collect();
        entries.sort();
        // Encode values
        for value in entries {
            offset += value.encode(buf, offset)?;
        }
        Ok(offset - offset_before)
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        const DATA_OFFSET: usize = 32;

        let aligned_offset = align_up::<ALIGN>(offset);

        // Check if there's enough data to read the header
        let header_end = aligned_offset
            .checked_add(Self::HEADER_SIZE)
            .ok_or(CodecError::Decoding(DecodingError::Overflow))?;

        if buf.remaining() < header_end {
            return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                expected: header_end,
                found: buf.remaining(),
                msg: "Not enough data to decode HashSet header".to_string(),
            }));
        }

        // Read data offset
        let data_offset = read_u32_aligned::<B, ALIGN>(buf, aligned_offset)? as usize;

        // Calculate start offset
        let start_offset = aligned_offset
            .checked_add(data_offset)
            .ok_or(CodecError::Decoding(DecodingError::Overflow))?;

        // Read length
        let length = read_u32_aligned::<B, ALIGN>(buf, start_offset)? as usize;
        if length == 0 {
            return Ok(HashSet::new());
        }

        // Read relative data offset (relative to the current offset)
        let values_offset = read_u32_aligned::<B, ALIGN>(buf, start_offset + DATA_OFFSET)? as usize;

        // Calculate absolute offset
        let values_start = values_offset
            .checked_add(start_offset)
            .and_then(|sum| sum.checked_add(DATA_OFFSET))
            .ok_or(CodecError::Decoding(DecodingError::Overflow))?;

        let mut result = HashSet::with_capacity(length);

        let values_data = &buf.chunk()[values_start + 32..];

        for i in 0..length {
            let value_offset = align_up::<ALIGN>(T::HEADER_SIZE)
                .checked_mul(i)
                .ok_or(CodecError::Decoding(DecodingError::Overflow))?;

            let value = T::decode(&values_data, value_offset)?;
            result.insert(value);
        }

        Ok(result)
    }

    fn partial_decode(buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        let aligned_offset = align_up::<ALIGN>(offset);
        let aligned_header_size = align_up::<ALIGN>(Self::HEADER_SIZE);

        if buf.remaining() < aligned_offset + aligned_header_size {
            return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                expected: aligned_offset + aligned_header_size,
                found: buf.remaining(),
                msg: "Not enough data to decode HashSet header".to_string(),
            }));
        }

        let data_offset = read_u32_aligned::<B, ALIGN>(buf, aligned_offset)? as usize;
        let start_offset = aligned_offset + data_offset;
        let length = read_u32_aligned::<B, ALIGN>(buf, start_offset)? as usize;
        let values_offset = read_u32_aligned::<B, ALIGN>(buf, start_offset + 64)? as usize;
        let values_start = start_offset + 64 + values_offset;

        let data_length = length * align_up::<ALIGN>(T::HEADER_SIZE);

        Ok((values_start + 32, data_length))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        encoder::{CompactABI, SolidityABI},
        test_utils::print_bytes,
    };
    use alloc::vec::Vec;
    use byteorder::BE;
    use bytes::BytesMut;
    use hashbrown::HashMap;

    #[test]
    fn test_nested_map() {
        let mut values = HashMap::new();
        values.insert(100, HashMap::from([(1, 2), (3, 4)]));
        values.insert(3, HashMap::new());
        values.insert(1000, HashMap::from([(7, 8), (9, 4)]));

        let mut buf = BytesMut::new();
        CompactABI::encode(&values, &mut buf).unwrap();

        let encoded = buf.freeze();
        let expected_encoded = "03000000140000000c000000200000005c0000000300000064000000e8030000000000003c000000000000003c00000000000000020000003c000000080000004400000008000000020000004c0000000800000054000000080000000100000003000000020000000400000007000000090000000800000004000000";

        assert_eq!(hex::encode(&encoded), expected_encoded, "Encoding mismatch");

        let decoded = CompactABI::<HashMap<i32, HashMap<i32, i32>>>::decode(&encoded, 0).unwrap();
        assert_eq!(values, decoded);

        let header =
            CompactABI::<HashMap<i32, HashMap<i32, i32>>>::partial_decode(&encoded, 0).unwrap();

        assert_eq!(header, (20, 104));
        println!("Header: {:?}", header);
    }

    #[test]
    fn test_vector_of_maps() {
        let values = vec![
            HashMap::from([(1, 2), (3, 4)]),
            HashMap::new(),
            HashMap::from([(7, 8), (9, 4)]),
        ];

        let mut buf = BytesMut::new();
        CompactABI::encode(&values, &mut buf).unwrap();

        let result = buf.freeze();
        println!("{}", hex::encode(&result));

        let expected_encoded = "030000000c0000005c000000020000003c000000080000004400000008000000000000004c000000000000004c00000000000000020000004c0000000800000054000000080000000100000003000000020000000400000007000000090000000800000004000000";

        assert_eq!(hex::encode(&result), expected_encoded, "Encoding mismatch");
        let bytes = result.clone();
        let values2 = CompactABI::<Vec<HashMap<u32, u32>>>::decode(&bytes, 0).unwrap();
        assert_eq!(values, values2);
    }

    #[test]
    fn test_map_of_vectors() {
        let mut values = HashMap::new();
        values.insert(vec![0, 1, 2], vec![3, 4, 5]);
        values.insert(vec![3, 1, 2], vec![3, 4, 5]);
        values.insert(vec![0, 1, 6], vec![3, 4, 5]);
        let mut buf = BytesMut::new();

        CompactABI::encode(&values, &mut buf).unwrap();
        let encoded = buf.freeze();

        // Note: The expected encoded string might need to be updated based on the new encoding
        // format
        let expected_encoded = "0300000014000000480000005c0000004800000003000000240000000c00000003000000300000000c000000030000003c0000000c00000000000000010000000200000000000000010000000600000003000000010000000200000003000000240000000c00000003000000300000000c000000030000003c0000000c000000030000000400000005000000030000000400000005000000030000000400000005000000";
        assert_eq!(hex::encode(&encoded), expected_encoded, "Encoding mismatch");

        let values2 = CompactABI::<HashMap<Vec<i32>, Vec<i32>>>::decode(&encoded, 0).unwrap();
        assert_eq!(values, values2);
    }

    #[test]
    fn test_set() {
        let values = HashSet::from([1, 2, 3]);
        let mut buf = BytesMut::new();

        CompactABI::encode(&values, &mut buf).unwrap();
        let encoded = buf.freeze();

        println!("{}", hex::encode(&encoded));
        let expected_encoded = "030000000c0000000c000000010000000200000003000000";
        assert_eq!(hex::encode(&encoded), expected_encoded, "Encoding mismatch");

        let values2 = CompactABI::<HashSet<i32>>::decode(&encoded, 0).unwrap();
        assert_eq!(values, values2);
    }

    #[test]
    fn test_set_is_sorted() {
        let values1 = HashSet::from([1, 2, 3, 4, 5, 6, 7, 8, 9]);
        let mut buf1 = BytesMut::new();

        CompactABI::encode(&values1, &mut buf1).unwrap();

        let values2 = HashSet::from([8, 3, 2, 4, 5, 9, 7, 1, 6]);
        let mut buf2 = BytesMut::new();

        CompactABI::encode(&values2, &mut buf2).unwrap();

        assert_eq!(&buf1.chunk(), &buf2.chunk());
    }

    #[test]
    fn test_set_solidity() {
        let values = HashSet::from([1, 2, 3]);
        let mut buf = BytesMut::new();
        SolidityABI::encode(&values, &mut buf).unwrap();
        let encoded = buf.freeze();
        print_bytes::<BE, 32>(&encoded);

        let values2 = SolidityABI::<HashSet<i32>>::decode(&encoded, 0).unwrap();
        println!("values2: {:?}", values2);
        assert_eq!(values, values2, "Decoding mismatch for Solidity");
    }

    #[test]
    fn test_set_solidity_is_sorted() {
        let values1 = HashSet::from([1, 2, 3, 4, 5, 6, 7, 8, 9]);
        let mut buf1 = BytesMut::new();

        SolidityABI::encode(&values1, &mut buf1).unwrap();

        let values2 = HashSet::from([8, 3, 2, 4, 5, 9, 7, 1, 6]);
        let mut buf2 = BytesMut::new();

        SolidityABI::encode(&values2, &mut buf2).unwrap();

        assert_eq!(
            &buf1.chunk(),
            &buf2.chunk(),
            "Solidity encoding is not sorted"
        );
    }
}
