use crate::{
    alloc::string::ToString,
    encoder::{align_up, read_u32_aligned, write_u32_aligned, Encoder},
    error::{CodecError, DecodingError},
};
use byteorder::ByteOrder;
use bytes::{Buf, BytesMut};
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct EmptyVec;

// Implementation for WASM mode (SOL_MODE = false)
impl<B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, false, false> for EmptyVec {
    const HEADER_SIZE: usize = size_of::<u32>() * 3; // 12 bytes
    const IS_DYNAMIC: bool = true;

    fn encode(&self, buf: &mut BytesMut, offset: usize) -> Result<(), CodecError> {
        let aligned_elem_size = align_up::<ALIGN>(4);

        // Write number of elements (0 for EmptyVec)
        write_u32_aligned::<B, ALIGN>(buf, offset, 0);

        // Write offset and length (both 0 for EmptyVec)
        write_u32_aligned::<B, ALIGN>(
            buf,
            offset + aligned_elem_size,
            (aligned_elem_size * 3) as u32,
        );
        write_u32_aligned::<B, ALIGN>(buf, offset + aligned_elem_size * 2, 0);

        Ok(())
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let aligned_elem_size = align_up::<ALIGN>(4);

        if buf.remaining() < offset + <Self as Encoder<B, ALIGN, false, false>>::HEADER_SIZE {
            return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                expected: offset + <Self as Encoder<B, ALIGN, false, false>>::HEADER_SIZE,
                found: buf.remaining(),
                msg: "failed to decode EmptyVec".to_string(),
            }));
        }

        let count = read_u32_aligned::<B, ALIGN>(buf, offset)?;
        if count != 0 {
            return Err(CodecError::Decoding(DecodingError::InvalidData(
                "EmptyVec must have count of 0".to_string(),
            )));
        }

        // Read and verify offset and length
        let data_offset = read_u32_aligned::<B, ALIGN>(buf, offset + aligned_elem_size)? as usize;
        let data_length =
            read_u32_aligned::<B, ALIGN>(buf, offset + aligned_elem_size * 2)? as usize;

        if data_offset != <Self as Encoder<B, ALIGN, false, false>>::HEADER_SIZE || data_length != 0
        {
            return Err(CodecError::Decoding(DecodingError::InvalidData(
                "Invalid offset or length for EmptyVec".to_string(),
            )));
        }

        Ok(EmptyVec)
    }

    fn partial_decode(buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        let aligned_elem_size = align_up::<ALIGN>(4);

        if buf.remaining() < offset + <Self as Encoder<B, ALIGN, false, false>>::HEADER_SIZE {
            return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                expected: offset + <Self as Encoder<B, ALIGN, false, false>>::HEADER_SIZE,
                found: buf.remaining(),
                msg: "failed to partially decode EmptyVec".to_string(),
            }));
        }

        let count = read_u32_aligned::<B, ALIGN>(buf, offset)?;
        if count != 0 {
            return Err(CodecError::Decoding(DecodingError::InvalidData(
                "EmptyVec must have count of 0".to_string(),
            )));
        }

        let data_offset = read_u32_aligned::<B, ALIGN>(buf, offset + aligned_elem_size)? as usize;
        let data_length =
            read_u32_aligned::<B, ALIGN>(buf, offset + aligned_elem_size * 2)? as usize;

        Ok((data_offset, data_length))
    }
}

// Implementation for Solidity mode (SOL_MODE = true)
impl<B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, true, false> for EmptyVec {
    const HEADER_SIZE: usize = 32; // Solidity uses 32 bytes for dynamic array header
    const IS_DYNAMIC: bool = true;

    fn encode(&self, buf: &mut BytesMut, offset: usize) -> Result<(), CodecError> {
        // Write offset to data
        write_u32_aligned::<B, ALIGN>(buf, offset, (offset + 32) as u32);

        // Write length (0 for EmptyVec)
        write_u32_aligned::<B, ALIGN>(buf, offset + 32, 0);

        Ok(())
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        if buf.remaining() < offset + 32 {
            return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                expected: offset + 32,
                found: buf.remaining(),
                msg: "failed to decode EmptyVec".to_string(),
            }));
        }

        let data_offset = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;
        let length = read_u32_aligned::<B, ALIGN>(buf, data_offset)? as usize;

        if length != 0 {
            return Err(CodecError::Decoding(DecodingError::InvalidData(
                "EmptyVec must have length of 0".to_string(),
            )));
        }

        Ok(EmptyVec)
    }

    fn partial_decode(buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        if buf.remaining() < offset + 32 {
            return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                expected: offset + 32,
                found: buf.remaining(),
                msg: "failed to partially decode EmptyVec".to_string(),
            }));
        }

        let data_offset = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;
        let length = read_u32_aligned::<B, ALIGN>(buf, data_offset)? as usize;

        Ok((data_offset, length))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CompactABI, SolidityABI};
    use byteorder::BigEndian;

    #[test]
    fn test_empty_vec_wasm_little_endian() {
        let empty_vec = EmptyVec;
        let mut buf = BytesMut::new();
        CompactABI::encode(&empty_vec, &mut buf, 0).unwrap();

        let encoded = buf.freeze();
        assert_eq!(hex::encode(&encoded), "000000000c00000000000000");

        let decoded = CompactABI::decode(&encoded, 0).unwrap();
        assert_eq!(empty_vec, decoded);

        let (offset, length) = CompactABI::<EmptyVec>::partial_decode(&encoded, 0).unwrap();
        assert_eq!(offset, 12);
        assert_eq!(length, 0);
    }

    #[test]
    fn test_empty_vec_wasm_big_endian() {
        let empty_vec = EmptyVec;
        let mut buf = BytesMut::new();
        <EmptyVec as Encoder<BigEndian, 4, false, false>>::encode(&empty_vec, &mut buf, 0).unwrap();

        let encoded = buf.freeze();
        assert_eq!(hex::encode(&encoded), "000000000000000c00000000");

        let decoded =
            <EmptyVec as Encoder<BigEndian, 4, false, false>>::decode(&encoded, 0).unwrap();
        assert_eq!(empty_vec, decoded);

        let (offset, length) =
            <EmptyVec as Encoder<BigEndian, 4, false, false>>::partial_decode(&encoded, 0).unwrap();
        assert_eq!(offset, 12);
        assert_eq!(length, 0);
    }

    #[test]
    fn test_empty_vec_solidity() {
        let empty_vec = EmptyVec;
        let mut buf = BytesMut::new();
        SolidityABI::encode(&empty_vec, &mut buf, 0).unwrap();

        let encoded = buf.freeze();

        assert_eq!(hex::encode(&encoded), "00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000000");

        let decoded = SolidityABI::decode(&encoded, 0).unwrap();
        assert_eq!(empty_vec, decoded);

        let (offset, length) = SolidityABI::<EmptyVec>::partial_decode(&encoded, 0).unwrap();
        assert_eq!(offset, 32);
        assert_eq!(length, 0);
    }

    #[test]
    fn test_empty_vec_wasm_with_offset() {
        let empty_vec = EmptyVec;
        let mut buf = BytesMut::from(&[0xFF, 0xFF, 0xFF][..]);
        CompactABI::encode(&empty_vec, &mut buf, 3).unwrap();

        let encoded = buf.freeze();
        println!("{}", hex::encode(&encoded));
        assert_eq!(hex::encode(&encoded), "ffffff000000000c00000000000000");

        let decoded = CompactABI::decode(&encoded, 3).unwrap();
        assert_eq!(empty_vec, decoded);

        let (offset, length) = CompactABI::<EmptyVec>::partial_decode(&encoded, 3).unwrap();
        assert_eq!(offset, 12);
        assert_eq!(length, 0);
    }
}
