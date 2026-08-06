use crate::{
    alloc::string::ToString,
    bytes_codec::{read_bytes, read_bytes_header, write_bytes_solidity, write_bytes_wasm},
    encoder::{
        align_up, checked_decode_slice_from, read_u32_aligned, validate_collection_body,
        write_u32_aligned, Encoder,
    },
    error::{CodecError, DecodingError},
};
use alloc::vec::Vec;
use byteorder::ByteOrder;
use bytes::{Buf, BytesMut};

/// We encode dynamic arrays as following:
/// - header
///   - length: number of elements inside vector
///   - offset: offset inside structure
///   - size: number of encoded bytes
/// - body
///   - raw bytes of the vector
///
/// For Solidity, we don't have size:
/// - header
///   - offset
/// - body
///   - length
///   - raw bytes of the vector
///
/// Implementation for non-Solidity mode
impl<T, B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, false, false> for Vec<T>
where
    T: Default + Sized + Encoder<B, ALIGN, false, false> + alloc::fmt::Debug,
{
    const HEADER_SIZE: usize = core::mem::size_of::<u32>() * 3;
    const IS_DYNAMIC: bool = true;

    fn encode(&self, buf: &mut BytesMut, offset: usize) -> Result<(), CodecError> {
        let aligned_elem_size = align_up::<ALIGN>(4);
        let aligned_header_size = aligned_elem_size * 3;

        // Ensure buffer can store header
        if buf.len() < offset + aligned_header_size {
            buf.resize(offset + aligned_header_size, 0);
        }

        // Write length of the vector
        write_u32_aligned::<B, ALIGN>(buf, offset, self.len() as u32);

        if self.is_empty() {
            // Write offset and size for empty vector
            write_u32_aligned::<B, ALIGN>(
                buf,
                offset + aligned_elem_size,
                aligned_header_size as u32,
            );
            write_u32_aligned::<B, ALIGN>(buf, offset + aligned_elem_size * 2, 0);
            return Ok(());
        }

        // Encode values
        let mut value_encoder = BytesMut::zeroed(ALIGN.max(T::HEADER_SIZE) * self.len());
        for (index, obj) in self.iter().enumerate() {
            let elem_offset = ALIGN.max(T::HEADER_SIZE) * index;
            obj.encode(&mut value_encoder, elem_offset)?;
        }

        let data = value_encoder.freeze();
        write_bytes_wasm::<B, ALIGN>(buf, offset + aligned_elem_size, &data);

        Ok(())
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let aligned_header_el_size = align_up::<ALIGN>(4);

        let header_end = offset
            .checked_add(aligned_header_el_size)
            .ok_or(CodecError::Decoding(DecodingError::Overflow))?;

        if buf.remaining() < header_end {
            return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                expected: header_end,
                found: buf.remaining(),
                msg: "failed to decode vector length".to_string(),
            }));
        }

        let data_len = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;
        if data_len == 0 {
            return Ok(Vec::new());
        }

        let data = read_bytes::<B, ALIGN, false>(buf, offset + aligned_header_el_size)?;
        let element_header_size = align_up::<ALIGN>(T::HEADER_SIZE);
        validate_collection_body(data_len, element_header_size, data.len())?;

        let mut result = Vec::new();
        result.try_reserve(data_len).map_err(|_| {
            CodecError::Decoding(DecodingError::InvalidData(
                "unable to reserve vector capacity".to_string(),
            ))
        })?;

        for i in 0..data_len {
            let elem_offset = i
                .checked_mul(element_header_size)
                .ok_or(CodecError::Decoding(DecodingError::Overflow))?;
            let value = T::decode(&data, elem_offset)?;
            result.push(value);
        }

        Ok(result)
    }

    fn partial_decode(buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        read_bytes_header::<B, ALIGN, false>(buf, offset)
    }
}

// Implementation for Solidity mode
impl<T, B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, true, false> for Vec<T>
where
    T: Default + Sized + Encoder<B, ALIGN, true, false> + alloc::fmt::Debug,
{
    const HEADER_SIZE: usize = 32;
    const IS_DYNAMIC: bool = true;

    fn encode(&self, buf: &mut BytesMut, offset: usize) -> Result<(), CodecError> {
        // Ensure buffer can store header
        if buf.len() < offset + Self::HEADER_SIZE {
            buf.resize(offset + Self::HEADER_SIZE, 0);
        }

        // Write offset
        write_u32_aligned::<B, ALIGN>(buf, offset, buf.len() as u32);

        if self.is_empty() {
            // Write length for empty vector
            write_u32_aligned::<B, ALIGN>(buf, buf.len(), 0);
            return Ok(());
        }

        // Encode values
        let mut value_encoder = BytesMut::zeroed(32 * self.len());
        for (index, obj) in self.iter().enumerate() {
            let elem_offset = ALIGN.max(T::HEADER_SIZE) * index;
            obj.encode(&mut value_encoder, elem_offset)?;
        }

        let data = value_encoder.freeze();
        write_bytes_solidity::<B, ALIGN>(buf, offset + Self::HEADER_SIZE, &data, self.len() as u32);

        Ok(())
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let data_offset = read_u32_aligned::<B, ALIGN>(buf, offset)?;
        let data_len = read_u32_aligned::<B, ALIGN>(buf, data_offset as usize)? as usize;

        if data_len == 0 {
            return Ok(Vec::new());
        }

        let body_offset = (data_offset as usize)
            .checked_add(32)
            .ok_or(CodecError::Decoding(DecodingError::Overflow))?;
        let chunk = checked_decode_slice_from(buf, body_offset, "vector body exceeds input")?;
        let element_header_size = align_up::<ALIGN>(T::HEADER_SIZE);
        validate_collection_body(data_len, element_header_size, chunk.len())?;

        let mut result = Vec::new();
        result.try_reserve(data_len).map_err(|_| {
            CodecError::Decoding(DecodingError::InvalidData(
                "unable to reserve vector capacity".to_string(),
            ))
        })?;

        for i in 0..data_len {
            let elem_offset = i
                .checked_mul(element_header_size)
                .ok_or(CodecError::Decoding(DecodingError::Overflow))?;
            let value = T::decode(&chunk, elem_offset)?;
            result.push(value);
        }

        Ok(result)
    }

    fn partial_decode(buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        read_bytes_header::<B, ALIGN, true>(buf, offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::{BigEndian, LittleEndian};
    use bytes::{Bytes, BytesMut};

    #[test]
    fn test_empty_vec_u32() {
        let original: Vec<u32> = Vec::new();
        let mut buf = BytesMut::new();

        <Vec<u32> as Encoder<LittleEndian, 4, false, false>>::encode(&original, &mut buf, 0)
            .unwrap();
        let encoded = buf.freeze();
        let expected = hex::decode("000000000c00000000000000").expect("Failed to decode hex");
        assert_eq!(encoded, Bytes::from(expected));

        let decoded =
            <Vec<u32> as Encoder<LittleEndian, 4, false, false>>::decode(&encoded, 0).unwrap();

        assert_eq!(original, decoded);
    }

    #[test]
    fn test_vec_u32_simple() {
        let original: Vec<u32> = vec![1, 2, 3, 4, 5];
        let mut buf = BytesMut::new();

        <Vec<u32> as Encoder<BigEndian, 4, false, false>>::encode(&original, &mut buf, 0).unwrap();
        let encoded = buf.freeze();

        let expected_encoded = "000000050000000c000000140000000100000002000000030000000400000005";
        assert_eq!(hex::encode(&encoded), expected_encoded);

        let decoded =
            <Vec<u32> as Encoder<BigEndian, 4, false, false>>::decode(&encoded, 0).unwrap();

        assert_eq!(original, decoded);
    }

    #[test]
    fn test_vec_u32_with_offset() {
        let original: Vec<u32> = vec![1, 2, 3, 4, 5];
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&[0xFF, 0xFF, 0xFF]); // Add some initial data

        <Vec<u32> as Encoder<LittleEndian, 4, false, false>>::encode(&original, &mut buf, 3)
            .unwrap();
        let encoded = buf.freeze();

        let decoded =
            <Vec<u32> as Encoder<LittleEndian, 4, false, false>>::decode(&encoded, 3).unwrap();

        assert_eq!(original, decoded);
    }

    #[test]
    fn test_vec_u8_with_offset() {
        let original: Vec<u8> = vec![1, 2, 3, 4, 5];
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&[0xFF, 0xFF, 0xFF]); // Add some initial data

        <Vec<u8> as Encoder<LittleEndian, 4, false, false>>::encode(&original, &mut buf, 3)
            .unwrap();
        let encoded = buf.freeze();

        let decoded =
            <Vec<u8> as Encoder<LittleEndian, 4, false, false>>::decode(&encoded, 3).unwrap();

        assert_eq!(original, decoded);
    }

    #[test]
    fn test_compact_vec_rejects_count_larger_than_body_before_allocation() {
        let encoded = Bytes::from_static(&[
            0xff, 0xff, 0xff, 0xff, // claimed element count
            0x0c, 0x00, 0x00, 0x00, // body offset
            0x00, 0x00, 0x00, 0x00, // body length
        ]);

        let error = <Vec<u32> as Encoder<LittleEndian, 4, false, false>>::decode(&encoded, 0)
            .expect_err("a count without element headers must fail before reserving capacity");

        assert!(matches!(
            error,
            CodecError::Decoding(DecodingError::BufferTooSmall { .. } | DecodingError::Overflow)
        ));
    }

    #[test]
    fn test_solidity_vec_rejects_count_larger_than_body_before_allocation() {
        let mut encoded = BytesMut::zeroed(64);
        encoded[28..32].copy_from_slice(&32_u32.to_be_bytes());
        encoded[60..64].copy_from_slice(&u32::MAX.to_be_bytes());

        let error = <Vec<u32> as Encoder<BigEndian, 32, true, false>>::decode(&encoded, 0)
            .expect_err("a count without element headers must fail before reserving capacity");

        assert!(matches!(
            error,
            CodecError::Decoding(DecodingError::BufferTooSmall { .. } | DecodingError::Overflow)
        ));
    }

    #[test]
    fn test_nested_vec_le_a2() {
        let original: Vec<Vec<u16>> = vec![vec![3, 4], vec![5, 6, 7]];

        let mut buf = BytesMut::new();
        <Vec<Vec<u16>> as Encoder<LittleEndian, 2, false, false>>::encode(&original, &mut buf, 0)
            .unwrap();
        let encoded = buf.freeze();

        let expected_encoded = "020000000c00000022000000020000001800000004000000030000001c0000000600000003000400050006000700";

        assert_eq!(hex::encode(&encoded), expected_encoded);

        let decoded =
            <Vec<Vec<u16>> as Encoder<LittleEndian, 2, false, false>>::decode(&encoded, 0).unwrap();

        assert_eq!(original, decoded);
    }

    #[test]
    fn test_nested_vec_a4_le() {
        let original: Vec<Vec<u16>> = vec![vec![3, 4], vec![5, 6, 7]];

        let mut buf = BytesMut::new();
        <Vec<Vec<u16>> as Encoder<LittleEndian, 4, false, false>>::encode(&original, &mut buf, 0)
            .unwrap();
        let encoded = buf.freeze();
        let decoded =
            <Vec<Vec<u16>> as Encoder<LittleEndian, 4, false, false>>::decode(&encoded, 0).unwrap();

        assert_eq!(original, decoded);
    }

    #[test]
    fn test_nested_vec_a4_be() {
        let original: Vec<Vec<u16>> = vec![vec![3, 4], vec![5, 6, 7]];

        let mut buf = BytesMut::new();
        <Vec<Vec<u16>> as Encoder<BigEndian, 4, false, false>>::encode(&original, &mut buf, 0)
            .unwrap();
        let encoded = buf.freeze();

        let decoded =
            <Vec<Vec<u16>> as Encoder<BigEndian, 4, false, false>>::decode(&encoded, 0).unwrap();

        assert_eq!(original, decoded);
    }

    #[test]
    fn test_large_vec() {
        let original: Vec<u64> = (0..1000).collect();
        let mut buf = BytesMut::new();

        <Vec<u64> as Encoder<BigEndian, 8, false, false>>::encode(&original, &mut buf, 0).unwrap();
        let encoded = buf.freeze();

        let decoded =
            <Vec<u64> as Encoder<BigEndian, 8, false, false>>::decode(&encoded, 0).unwrap();

        assert_eq!(original, decoded);
    }

    // New test for Solidity mode
    #[test]
    fn test_vec_u32_solidity_mode() {
        let original: Vec<u32> = vec![1, 2, 3, 4, 5];
        let mut buf = BytesMut::new();

        <Vec<u32> as Encoder<BigEndian, 32, true, false>>::encode(&original, &mut buf, 0).unwrap();
        let encoded = buf.freeze();

        let decoded =
            <Vec<u32> as Encoder<BigEndian, 32, true, false>>::decode(&encoded, 0).unwrap();

        assert_eq!(original, decoded);
    }
}
