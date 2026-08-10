use crate::{
    bytes_codec::{read_bytes_header, write_bytes, write_bytes_solidity, write_bytes_wasm},
    encoder::{
        align_up, checked_decode_slice, checked_decode_slice_from, read_u32_aligned,
        validate_collection_body, write_u32_aligned, Encoder,
    },
    error::{CodecError, DecodingError},
};
use alloc::{format, string::ToString, vec::Vec};
use byteorder::ByteOrder;
use bytes::{Buf, BytesMut};
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

    fn encode(&self, buf: &mut BytesMut, offset: usize) -> Result<(), CodecError> {
        let aligned_header_el_size = align_up::<ALIGN>(4);
        let aligned_header_size = align_up::<ALIGN>(Self::HEADER_SIZE);

        // Ensure buf is large enough for the header
        if buf.len() < offset + aligned_header_size {
            buf.resize(offset + aligned_header_size, 0);
        }

        // Write map size
        write_u32_aligned::<B, ALIGN>(buf, offset, self.len() as u32);

        // Make sure keys & values are sorted
        let mut entries: Vec<_> = self.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));

        // Encode and write keys
        let mut key_buf = BytesMut::zeroed(align_up::<ALIGN>(K::HEADER_SIZE) * self.len());

        for (i, (key, _)) in entries.iter().enumerate() {
            let key_offset = align_up::<ALIGN>(K::HEADER_SIZE) * i;
            key.encode(&mut key_buf, key_offset)?;
        }

        // write keys header and keys data
        write_bytes::<B, ALIGN, false>(
            buf,
            offset + aligned_header_el_size,
            &key_buf,
            entries.len() as u32,
        );

        // Encode and write values
        let mut value_buf = BytesMut::zeroed(align_up::<ALIGN>(V::HEADER_SIZE) * self.len());
        for (i, (_, value)) in entries.iter().enumerate() {
            let value_offset = align_up::<ALIGN>(V::HEADER_SIZE) * i;
            value.encode(&mut value_buf, value_offset)?;
        }

        write_bytes_wasm::<B, ALIGN>(buf, offset + aligned_header_el_size * 3, &value_buf);

        Ok(())
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let aligned_header_el_size = align_up::<ALIGN>(4);
        let aligned_header_size = align_up::<ALIGN>(Self::HEADER_SIZE);

        let header_end = offset
            .checked_add(aligned_header_size)
            .ok_or(CodecError::Decoding(DecodingError::Overflow))?;

        if buf.remaining() < header_end {
            return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                expected: header_end,
                found: buf.remaining(),
                msg: "Not enough data to decode HashMap header".to_string(),
            }));
        }

        let length = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;

        let keys_header_offset = offset
            .checked_add(aligned_header_el_size)
            .ok_or(CodecError::Decoding(DecodingError::Overflow))?;
        let (keys_offset, keys_length) =
            read_bytes_header::<B, ALIGN, false>(buf, keys_header_offset)?;

        let values_header_offset = aligned_header_el_size
            .checked_mul(3)
            .and_then(|header_size| offset.checked_add(header_size))
            .ok_or(CodecError::Decoding(DecodingError::Overflow))?;
        let (values_offset, values_length) =
            read_bytes_header::<B, ALIGN, false>(buf, values_header_offset)?;

        let key_bytes = checked_decode_slice(buf, keys_offset, keys_length, "keys exceed input")?;
        let value_bytes =
            checked_decode_slice(buf, values_offset, values_length, "values exceed input")?;
        let key_header_size = align_up::<ALIGN>(K::HEADER_SIZE);
        let value_header_size = align_up::<ALIGN>(V::HEADER_SIZE);
        validate_collection_body(length, key_header_size, key_bytes.len())?;
        validate_collection_body(length, value_header_size, value_bytes.len())?;

        let mut result = HashMap::new();
        result.try_reserve(length).map_err(|_| {
            CodecError::Decoding(DecodingError::InvalidData(
                "unable to reserve map capacity".to_string(),
            ))
        })?;

        for i in 0..length {
            let key_offset = key_header_size
                .checked_mul(i)
                .ok_or(CodecError::Decoding(DecodingError::Overflow))?;
            let value_offset = value_header_size
                .checked_mul(i)
                .ok_or(CodecError::Decoding(DecodingError::Overflow))?;
            let key = K::decode(&key_bytes, key_offset)?;
            let value = V::decode(&value_bytes, value_offset)?;
            result.insert(key, value);
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

    fn encode(&self, buf: &mut BytesMut, offset: usize) -> Result<(), CodecError> {
        // Ensure buf is large enough for the header
        if buf.len() < offset + Self::HEADER_SIZE {
            buf.resize(offset + Self::HEADER_SIZE, 0);
        }

        // Write offset size
        write_u32_aligned::<B, ALIGN>(buf, offset, 32_u32);

        // Write map size
        write_u32_aligned::<B, ALIGN>(buf, offset + 32, self.len() as u32);

        // Make sure keys & values are sorted
        let mut entries: Vec<_> = self.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));

        // Encode and write keys
        let mut key_buf = BytesMut::zeroed(align_up::<ALIGN>(K::HEADER_SIZE) * self.len());

        for (i, (key, _)) in entries.iter().enumerate() {
            let key_offset = align_up::<ALIGN>(K::HEADER_SIZE) * i;
            key.encode(&mut key_buf, key_offset)?;
        }
        let relative_key_offset = buf.len() - offset - 64;
        // Write key offset
        write_u32_aligned::<B, ALIGN>(buf, offset + 64, relative_key_offset as u32);

        // write key header and keys data to the buf
        write_bytes_solidity::<B, ALIGN>(buf, offset + 64, &key_buf, entries.len() as u32);

        // Write values offset
        let relative_value_offset = buf.len() - offset - 96;
        write_u32_aligned::<B, ALIGN>(buf, offset + 96, relative_value_offset as u32);

        // Encode and write values
        let mut value_buf = BytesMut::zeroed(align_up::<ALIGN>(V::HEADER_SIZE) * self.len());
        for (i, (_, value)) in entries.iter().enumerate() {
            let value_offset = align_up::<ALIGN>(V::HEADER_SIZE) * i;
            value.encode(&mut value_buf, value_offset)?;
        }

        write_bytes_solidity::<B, ALIGN>(buf, buf.len(), &value_buf, entries.len() as u32);

        Ok(())
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
        let keys_offset_position = start_offset
            .checked_add(KEYS_OFFSET)
            .ok_or(CodecError::Decoding(DecodingError::Overflow))?;
        let keys_offset = read_u32_aligned::<B, ALIGN>(buf, keys_offset_position)? as usize;
        let values_offset_position = start_offset
            .checked_add(VALUES_OFFSET)
            .ok_or(CodecError::Decoding(DecodingError::Overflow))?;
        let values_offset = read_u32_aligned::<B, ALIGN>(buf, values_offset_position)? as usize;

        // Calculate absolute offsets
        let keys_start = keys_offset
            .checked_add(start_offset)
            .and_then(|sum| sum.checked_add(KEYS_OFFSET))
            .ok_or(CodecError::Decoding(DecodingError::Overflow))?;
        let values_start = values_offset
            .checked_add(start_offset)
            .and_then(|sum| sum.checked_add(VALUES_OFFSET))
            .ok_or(CodecError::Decoding(DecodingError::Overflow))?;

        let keys_data_start = keys_start
            .checked_add(32)
            .ok_or(CodecError::Decoding(DecodingError::Overflow))?;
        let values_data_start = values_start
            .checked_add(32)
            .ok_or(CodecError::Decoding(DecodingError::Overflow))?;
        let keys_data = checked_decode_slice_from(buf, keys_data_start, "keys body exceeds input")?;
        let values_data =
            checked_decode_slice_from(buf, values_data_start, "values body exceeds input")?;
        let key_header_size = align_up::<ALIGN>(K::HEADER_SIZE);
        let value_header_size = align_up::<ALIGN>(V::HEADER_SIZE);
        validate_collection_body(length, key_header_size, keys_data.len())?;
        validate_collection_body(length, value_header_size, values_data.len())?;

        let mut result = HashMap::new();
        result.try_reserve(length).map_err(|_| {
            CodecError::Decoding(DecodingError::InvalidData(
                "unable to reserve map capacity".to_string(),
            ))
        })?;

        for i in 0..length {
            let key_offset = key_header_size
                .checked_mul(i)
                .ok_or(CodecError::Decoding(DecodingError::Overflow))?;
            let value_offset = value_header_size
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

        // Where the length word sits, and the number of entries stored there - the same pair a
        // `Vec` reports in this mode. A map writes its body offset *relative to its own head*
        // while a `Vec` writes an absolute buffer position, so the relative value has to be
        // rebased on `offset`; reading it as absolute is only right when the map starts at zero.
        //
        // The previous version read the *compact* header layout from inside this Solidity impl,
        // at `align_up::<ALIGN>(4)` and `align_up::<ALIGN>(12)` - which at ALIGN 32 are both 32,
        // so it read one place twice and returned the entry count as the offset.
        let body = offset
            .checked_add(read_u32_aligned::<B, ALIGN>(buf, offset)? as usize)
            .ok_or(CodecError::Decoding(DecodingError::Overflow))?;
        let length = read_u32_aligned::<B, ALIGN>(buf, body)? as usize;

        Ok((body, length))
    }
}

/// Implement encoding for HashSet, SOL_MODE = false
impl<T, B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, false, false> for HashSet<T>
where
    T: Default + Sized + Encoder<B, ALIGN, false, false> + Eq + Hash + Ord,
{
    const HEADER_SIZE: usize = 4 + 8; // length + data_header
    const IS_DYNAMIC: bool = true;

    fn encode(&self, buf: &mut BytesMut, offset: usize) -> Result<(), CodecError> {
        let aligned_offset = align_up::<ALIGN>(offset);
        let aligned_header_el_size = align_up::<ALIGN>(4);
        let aligned_header_size = align_up::<ALIGN>(Self::HEADER_SIZE);

        // Ensure buf is large enough for the header
        if buf.len() < aligned_offset + aligned_header_size {
            buf.resize(aligned_offset + aligned_header_size, 0);
        }

        // Write set size
        write_u32_aligned::<B, ALIGN>(buf, aligned_offset, self.len() as u32);

        // Make sure a set is sorted
        let mut entries: Vec<_> = self.iter().collect();
        entries.sort();

        // Encode values
        let mut value_buf = BytesMut::zeroed(align_up::<ALIGN>(T::HEADER_SIZE) * self.len());
        for (i, value) in entries.iter().enumerate() {
            let value_offset = align_up::<ALIGN>(T::HEADER_SIZE) * i;
            value.encode(&mut value_buf, value_offset)?;
        }

        // Write values
        write_bytes::<B, ALIGN, false>(
            buf,
            aligned_offset + aligned_header_el_size,
            &value_buf,
            entries.len() as u32,
        );

        Ok(())
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let aligned_offset = align_up::<ALIGN>(offset);
        let aligned_header_size = align_up::<ALIGN>(Self::HEADER_SIZE);

        let header_end = aligned_offset
            .checked_add(aligned_header_size)
            .ok_or(CodecError::Decoding(DecodingError::Overflow))?;

        if buf.remaining() < header_end {
            return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                expected: header_end,
                found: buf.remaining(),
                msg: "Not enough data to decode HashSet header".to_string(),
            }));
        }

        let length = read_u32_aligned::<B, ALIGN>(buf, aligned_offset)? as usize;

        let data_header_offset = aligned_offset
            .checked_add(align_up::<ALIGN>(4))
            .ok_or(CodecError::Decoding(DecodingError::Overflow))?;
        let (data_offset, data_length) =
            read_bytes_header::<B, ALIGN, false>(buf, data_header_offset)?;

        let value_bytes =
            checked_decode_slice(buf, data_offset, data_length, "values exceed input")?;
        let value_header_size = align_up::<ALIGN>(T::HEADER_SIZE);
        validate_collection_body(length, value_header_size, value_bytes.len())?;

        let mut result = HashSet::new();
        result.try_reserve(length).map_err(|_| {
            CodecError::Decoding(DecodingError::InvalidData(
                "unable to reserve set capacity".to_string(),
            ))
        })?;

        for i in 0..length {
            let value_offset = value_header_size
                .checked_mul(i)
                .ok_or(CodecError::Decoding(DecodingError::Overflow))?;
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
    fn encode(&self, buf: &mut BytesMut, offset: usize) -> Result<(), CodecError> {
        let aligned_offset = align_up::<ALIGN>(offset);

        // Ensure buf is large enough for the header
        if buf.len() < aligned_offset + Self::HEADER_SIZE {
            buf.resize(aligned_offset + Self::HEADER_SIZE, 0);
        }

        // Write offset size
        write_u32_aligned::<B, ALIGN>(buf, aligned_offset, 32_u32);

        // Write set size
        write_u32_aligned::<B, ALIGN>(buf, aligned_offset + 32, self.len() as u32);

        // Make sure set is sorted
        let mut entries: Vec<_> = self.iter().collect();
        entries.sort();

        // Encode values
        let mut value_buf = BytesMut::zeroed(align_up::<ALIGN>(T::HEADER_SIZE) * self.len());
        for (i, value) in entries.iter().enumerate() {
            let value_offset = align_up::<ALIGN>(T::HEADER_SIZE) * i;
            value.encode(&mut value_buf, value_offset)?;
        }

        // Write data offset
        let relative_data_offset = buf.len() - aligned_offset - 64;
        write_u32_aligned::<B, ALIGN>(buf, aligned_offset + 64, relative_data_offset as u32);

        // Write values
        write_bytes_solidity::<B, ALIGN>(buf, buf.len(), &value_buf, entries.len() as u32);

        Ok(())
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
        let values_offset_position = start_offset
            .checked_add(DATA_OFFSET)
            .ok_or(CodecError::Decoding(DecodingError::Overflow))?;
        let values_offset = read_u32_aligned::<B, ALIGN>(buf, values_offset_position)? as usize;

        // Calculate absolute offset
        let values_start = values_offset
            .checked_add(start_offset)
            .and_then(|sum| sum.checked_add(DATA_OFFSET))
            .ok_or(CodecError::Decoding(DecodingError::Overflow))?;

        let values_data_start = values_start
            .checked_add(32)
            .ok_or(CodecError::Decoding(DecodingError::Overflow))?;
        let values_data =
            checked_decode_slice_from(buf, values_data_start, "values body exceeds input")?;
        let value_header_size = align_up::<ALIGN>(T::HEADER_SIZE);
        validate_collection_body(length, value_header_size, values_data.len())?;

        let mut result = HashSet::new();
        result.try_reserve(length).map_err(|_| {
            CodecError::Decoding(DecodingError::InvalidData(
                "unable to reserve set capacity".to_string(),
            ))
        })?;

        for i in 0..length {
            let value_offset = value_header_size
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

        // Where the length word sits, and how many elements are stored there. As with `HashMap`,
        // the body offset is relative to this head, so it is rebased on `aligned_offset`.
        //
        // The previous version read the region offset from `start_offset + 64`, where the layout
        // puts the region's own count rather than its offset, and reported an unaligned position
        // derived from it.
        let body = aligned_offset
            .checked_add(read_u32_aligned::<B, ALIGN>(buf, aligned_offset)? as usize)
            .ok_or(CodecError::Decoding(DecodingError::Overflow))?;
        let length = read_u32_aligned::<B, ALIGN>(buf, body)? as usize;

        Ok((body, length))
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
    use bytes::{Bytes, BytesMut};
    #[test]
    fn test_compact_map_rejects_count_larger_than_bodies_before_allocation() {
        let encoded = Bytes::from_static(&[
            0xff, 0xff, 0xff, 0xff, // claimed entry count
            0x14, 0x00, 0x00, 0x00, // keys body offset
            0x00, 0x00, 0x00, 0x00, // keys body length
            0x14, 0x00, 0x00, 0x00, // values body offset
            0x00, 0x00, 0x00, 0x00, // values body length
        ]);

        let error = CompactABI::<HashMap<u32, u32>>::decode(&encoded, 0)
            .expect_err("a count without key/value headers must fail before reserving capacity");

        assert!(matches!(
            error,
            CodecError::Decoding(DecodingError::BufferTooSmall { .. } | DecodingError::Overflow)
        ));
    }

    #[test]
    fn test_compact_set_rejects_count_larger_than_body_before_allocation() {
        let encoded = Bytes::from_static(&[
            0xff, 0xff, 0xff, 0xff, // claimed entry count
            0x0c, 0x00, 0x00, 0x00, // values body offset
            0x00, 0x00, 0x00, 0x00, // values body length
        ]);

        let error = CompactABI::<HashSet<u32>>::decode(&encoded, 0)
            .expect_err("a count without value headers must fail before reserving capacity");

        assert!(matches!(
            error,
            CodecError::Decoding(DecodingError::BufferTooSmall { .. } | DecodingError::Overflow)
        ));
    }

    #[test]
    fn test_solidity_map_rejects_count_larger_than_bodies_before_allocation() {
        let mut encoded = BytesMut::zeroed(160);
        encoded[28..32].copy_from_slice(&32_u32.to_be_bytes());
        encoded[60..64].copy_from_slice(&u32::MAX.to_be_bytes());
        encoded[92..96].copy_from_slice(&32_u32.to_be_bytes());
        encoded[124..128].copy_from_slice(&32_u32.to_be_bytes());

        let error = SolidityABI::<HashMap<u32, u32>>::decode(&encoded, 0)
            .expect_err("an oversized count must fail before reserving map capacity");

        assert!(matches!(
            error,
            CodecError::Decoding(DecodingError::BufferTooSmall { .. } | DecodingError::Overflow)
        ));
    }

    #[test]
    fn test_solidity_set_rejects_count_larger_than_body_before_allocation() {
        let mut encoded = BytesMut::zeroed(128);
        encoded[28..32].copy_from_slice(&32_u32.to_be_bytes());
        encoded[60..64].copy_from_slice(&u32::MAX.to_be_bytes());
        encoded[92..96].copy_from_slice(&32_u32.to_be_bytes());

        let error = SolidityABI::<HashSet<u32>>::decode(&encoded, 0)
            .expect_err("an oversized count must fail before reserving set capacity");

        assert!(matches!(
            error,
            CodecError::Decoding(DecodingError::BufferTooSmall { .. } | DecodingError::Overflow)
        ));
    }

    #[test]
    fn test_nested_map() {
        let mut values = HashMap::new();
        values.insert(100, HashMap::from([(1, 2), (3, 4)]));
        values.insert(3, HashMap::new());
        values.insert(1000, HashMap::from([(7, 8), (9, 4)]));

        let mut buf = BytesMut::new();
        CompactABI::encode(&values, &mut buf, 0).unwrap();

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
        CompactABI::encode(&values, &mut buf, 0).unwrap();

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

        CompactABI::encode(&values, &mut buf, 0).unwrap();
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

        CompactABI::encode(&values, &mut buf, 0).unwrap();
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

        CompactABI::encode(&values1, &mut buf1, 0).unwrap();

        let values2 = HashSet::from([8, 3, 2, 4, 5, 9, 7, 1, 6]);
        let mut buf2 = BytesMut::new();

        CompactABI::encode(&values2, &mut buf2, 0).unwrap();

        assert_eq!(&buf1.chunk(), &buf2.chunk());
    }

    #[test]
    fn test_set_solidity() {
        let values = HashSet::from([1, 2, 3]);
        let mut buf = BytesMut::new();
        SolidityABI::encode(&values, &mut buf, 0).unwrap();
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

        SolidityABI::encode(&values1, &mut buf1, 0).unwrap();

        let values2 = HashSet::from([8, 3, 2, 4, 5, 9, 7, 1, 6]);
        let mut buf2 = BytesMut::new();

        SolidityABI::encode(&values2, &mut buf2, 0).unwrap();

        assert_eq!(
            &buf1.chunk(),
            &buf2.chunk(),
            "Solidity encoding is not sorted"
        );
    }
}
