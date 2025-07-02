#![allow(warnings)]
use crate::{
    alloc::string::ToString,
    bytes_codec::{read_bytes, read_bytes_header, write_bytes},
    encoder::{align_up, is_big_endian, write_aligned_slice, write_u32_aligned, Encoder},
    error::{CodecError, DecodingError},
};
use alloc::string::String;
use alloy_primitives::{Address, Bytes, FixedBytes, Signed, Uint};
use byteorder::ByteOrder;
use bytes::{Buf, BufMut, BytesMut};

impl<B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, true, false> for Bytes {
    const HEADER_SIZE: usize = 32;
    const IS_DYNAMIC: bool = true;

    /// Encode the bytes into the buffer for Solidity mode.
    /// First, we encode the header and write it to the given offset.
    /// After that, we encode the actual data and write it to the end of the buffer.
    fn encode(&self, buf: &mut impl BufMut, mut offset: usize) -> Result<usize, CodecError> {
        // Write the offset of the data (current length of the buffer)
        offset += write_u32_aligned::<B, ALIGN>(buf, offset as u32);
        // Write the actual data to the buffer at the current length
        offset += write_bytes::<B, ALIGN, true>(buf, offset + ALIGN, self, self.len() as u32);
        // Add padding if necessary to ensure the buffer remains aligned
        let unpadded_bytes = self.len() % ALIGN;
        if unpadded_bytes != 0 {
            buf.put_bytes(0, ALIGN - unpadded_bytes);
            offset += ALIGN - unpadded_bytes;
        }
        Ok(offset)
    }

    /// Decode the bytes from the buffer for Solidity mode.
    /// Reads the header to get the data offset and size, then reads the actual data.
    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        Ok(Self::from(read_bytes::<B, ALIGN, true>(buf, offset)?))
    }

    /// Partially decode the bytes from the buffer for Solidity mode.
    /// Returns the data offset and size without reading the actual data.
    fn partial_decode(buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        read_bytes_header::<B, ALIGN, true>(buf, offset)
    }
}

impl<B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, false, false> for Bytes {
    const HEADER_SIZE: usize = size_of::<u32>() * 2;
    const IS_DYNAMIC: bool = true;

    /// Encode the bytes into the buffer.
    /// First, we encode the header and write it to the given offset.
    /// After that, we encode the actual data and write it to the end of the buffer.
    fn encode(&self, buf: &mut impl BufMut, mut offset: usize) -> Result<usize, CodecError> {
        offset += write_bytes::<B, ALIGN, false>(buf, offset, self, self.len() as u32);
        // Add padding if necessary to ensure the buffer remains aligned
        let unpadded_bytes = self.len() % ALIGN;
        if unpadded_bytes != 0 {
            buf.put_bytes(0, ALIGN - unpadded_bytes);
            offset += ALIGN - unpadded_bytes;
        }
        Ok(offset)
    }

    /// Decode the bytes from the buffer.
    /// Reads the header to get the data offset and size, then read the actual data.
    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        Ok(Self::from(read_bytes::<B, ALIGN, false>(buf, offset)?))
    }

    /// Partially decode the bytes from the buffer.
    /// Returns the data offset and size without reading the actual data.
    fn partial_decode(buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        read_bytes_header::<B, ALIGN, false>(buf, offset)
    }
}

impl<B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, true, false> for String {
    const HEADER_SIZE: usize = <Bytes as Encoder<B, ALIGN, true, false>>::HEADER_SIZE;
    const IS_DYNAMIC: bool = true;

    fn encode(&self, buf: &mut impl BufMut, offset: usize) -> Result<usize, CodecError> {
        <Bytes as Encoder<B, ALIGN, true, false>>::encode(
            &Bytes::copy_from_slice(self.as_bytes()),
            buf,
            offset,
        )
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let bytes = <Bytes as Encoder<B, ALIGN, true, false>>::decode(buf, offset)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| {
            CodecError::Decoding(DecodingError::InvalidData(
                "failed to decode string from utf8".to_string(),
            ))
        })
    }

    fn partial_decode(buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        <Bytes as Encoder<B, ALIGN, true, false>>::partial_decode(buf, offset)
    }
}

impl<B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, false, false> for String {
    const HEADER_SIZE: usize = <Bytes as Encoder<B, ALIGN, false, false>>::HEADER_SIZE;
    const IS_DYNAMIC: bool = true;

    fn encode(&self, buf: &mut impl BufMut, offset: usize) -> Result<usize, CodecError> {
        <Bytes as Encoder<B, ALIGN, false, false>>::encode(
            &Bytes::copy_from_slice(self.as_bytes()),
            buf,
            offset,
        )
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let bytes = <Bytes as Encoder<B, ALIGN, false, false>>::decode(buf, offset)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| {
            CodecError::Decoding(DecodingError::InvalidData(
                "failed to decode string from utf8".to_string(),
            ))
        })
    }

    fn partial_decode(buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        <Bytes as Encoder<B, ALIGN, false, false>>::partial_decode(buf, offset)
    }
}

impl<const N: usize, B: ByteOrder, const ALIGN: usize, const IS_STATIC: bool>
    Encoder<B, ALIGN, false, IS_STATIC> for FixedBytes<N>
{
    const HEADER_SIZE: usize = N;
    const IS_DYNAMIC: bool = false;

    /// Encode the fixed bytes into the buffer.
    /// Writes the fixed bytes directly to the buffer at the given offset.
    fn encode(&self, buf: &mut impl BufMut, mut offset: usize) -> Result<usize, CodecError> {
        offset += write_aligned_slice::<B, N>(buf, &self.0);
        Ok(offset)
    }

    /// Decode the fixed bytes from the buffer.
    /// Reads the fixed bytes directly from the buffer at the given offset.
    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        if buf.remaining() < offset + N {
            return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                expected: offset + N,
                found: buf.remaining(),
                msg: "Buffer too small to decode FixedBytes".to_string(),
            }));
        }
        let data = buf.chunk()[offset..offset + N].to_vec();
        Ok(FixedBytes::from_slice(&data))
    }

    /// Partially decode the fixed bytes from the buffer.
    /// Returns the data offset and size without reading the actual data.
    fn partial_decode(_buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        let aligned_offset = align_up::<ALIGN>(offset);
        Ok((aligned_offset, N))
    }
}

impl<const N: usize, B: ByteOrder, const ALIGN: usize, const IS_STATIC: bool>
    Encoder<B, ALIGN, true, IS_STATIC> for FixedBytes<N>
{
    const HEADER_SIZE: usize = N; // Always 32 bytes for Solidity ABI
    const IS_DYNAMIC: bool = false;

    /// Encode the fixed bytes into the buffer for Solidity mode.
    /// Writes the fixed bytes directly to the buffer at the given offset, zero-padding to 32 bytes.
    fn encode(&self, buf: &mut impl BufMut, mut offset: usize) -> Result<usize, CodecError> {
        offset += write_aligned_slice::<B, N>(buf, &self.0);
        Ok(offset)
    }

    /// Decode the fixed bytes from the buffer for Solidity mode.
    /// Reads the fixed bytes directly from the buffer at the given offset, assuming 32-byte
    /// alignment.
    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let offset = align_up::<32>(offset); // Always 32-byte aligned for Solidity
        if buf.remaining() < offset + 32 {
            return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                expected: offset + 32,
                found: buf.remaining(),
                msg: "Buffer too small to decode FixedBytes".to_string(),
            }));
        }
        let data = buf.chunk()[offset..offset + N].to_vec();
        Ok(FixedBytes::from_slice(&data))
    }

    /// Partially decode the fixed bytes from the buffer for Solidity mode.
    /// Returns the data offset and size without reading the actual data.
    fn partial_decode(_buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        Ok((offset, 32))
    }
}

macro_rules! impl_evm_fixed {
    ($type:ty) => {
        impl<B: ByteOrder, const ALIGN: usize, const IS_STATIC: bool>
            Encoder<B, ALIGN, false, IS_STATIC> for $type
        {
            const HEADER_SIZE: usize = <$type>::len_bytes();
            const IS_DYNAMIC: bool = false;

            /// Encode the fixed bytes into the buffer.
            /// Writes the fixed bytes directly to the buffer at the given offset.
            fn encode(
                &self,
                buf: &mut impl BufMut,
                mut offset: usize,
            ) -> Result<usize, CodecError> {
                offset += write_aligned_slice::<B, ALIGN>(buf, self.as_slice());
                Ok(offset)
            }

            /// Decode the fixed bytes from the buffer.
            /// Reads the fixed bytes directly from the buffer at the given offset.
            fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
                let size = <$type>::len_bytes();
                if buf.remaining() < offset + size {
                    return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                        expected: offset + size,
                        found: buf.remaining(),
                        msg: "Buffer too small to decode fixed bytes".to_string(),
                    }));
                }
                let data = buf.chunk()[offset..offset + size].to_vec();
                Ok(<$type>::from_slice(&data))
            }

            /// Partially decode the fixed bytes from the buffer.
            /// Returns the data offset and size without reading the actual data.
            fn partial_decode(
                _buf: &impl Buf,
                offset: usize,
            ) -> Result<(usize, usize), CodecError> {
                Ok((offset, <$type>::len_bytes()))
            }
        }

        impl<B: ByteOrder, const ALIGN: usize, const IS_STATIC: bool>
            Encoder<B, ALIGN, true, IS_STATIC> for $type
        {
            const HEADER_SIZE: usize = 32; // Always 32 bytes for Solidity ABI
            const IS_DYNAMIC: bool = false;

            /// Encode the fixed bytes into the buffer for Solidity mode.
            /// Writes the fixed bytes directly to the buffer at the given offset, zero-padding to
            /// 32 bytes.
            fn encode(
                &self,
                buf: &mut impl BufMut,
                mut offset: usize,
            ) -> Result<usize, CodecError> {
                offset += write_aligned_slice::<B, ALIGN>(buf, self.as_slice());
                Ok(offset)
            }

            /// Decode the fixed bytes from the buffer for Solidity mode.
            /// Reads the fixed bytes directly from the buffer at the given offset, assuming 32-byte
            /// alignment.
            fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
                let size = <$type>::len_bytes();
                if buf.remaining() < offset + 32 {
                    return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                        expected: offset + 32,
                        found: buf.remaining(),
                        msg: "Buffer too small to decode fixed bytes".to_string(),
                    }));
                }
                let data = buf.chunk()[offset + 32 - size..offset + 32].to_vec();
                Ok(<$type>::from_slice(&data))
            }

            /// Partially decode the fixed bytes from the buffer for Solidity mode.
            /// Returns the data offset and size without reading the actual data.
            fn partial_decode(
                _buf: &impl Buf,
                offset: usize,
            ) -> Result<(usize, usize), CodecError> {
                Ok((offset, 32))
            }
        }
    };
}

impl_evm_fixed!(Address);

impl<
        const BITS: usize,
        const LIMBS: usize,
        B: ByteOrder,
        const ALIGN: usize,
        const IS_STATIC: bool,
    > Encoder<B, ALIGN, false, IS_STATIC> for Uint<BITS, LIMBS>
{
    const HEADER_SIZE: usize = Self::BYTES;
    const IS_DYNAMIC: bool = false;

    fn encode(&self, buf: &mut impl BufMut, mut offset: usize) -> Result<usize, CodecError> {
        let bytes = if is_big_endian::<B>() {
            self.to_be_bytes::<32>()
        } else {
            self.to_le_bytes::<32>()
        };
        offset += write_aligned_slice::<B, ALIGN>(buf, &bytes);
        Ok(offset)
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let word_size = align_up::<ALIGN>(Self::BYTES);

        if buf.remaining() < offset + word_size {
            return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                expected: offset + word_size,
                found: buf.remaining(),
                msg: "buf too small to read Uint".to_string(),
            }));
        }

        let chunk = &buf.chunk()[offset..offset + word_size];
        let value_slice = &chunk[..Self::BYTES];

        let value = if is_big_endian::<B>() {
            Self::from_be_slice(value_slice)
        } else {
            Self::from_le_slice(value_slice)
        };

        Ok(value)
    }

    fn partial_decode(_buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        let word_size = align_up::<ALIGN>(Self::BYTES);
        Ok((offset, word_size))
    }
}

impl<
        const BITS: usize,
        const LIMBS: usize,
        B: ByteOrder,
        const ALIGN: usize,
        const IS_STATIC: bool,
    > Encoder<B, ALIGN, true, IS_STATIC> for Uint<BITS, LIMBS>
{
    const HEADER_SIZE: usize = 32; // Always 32 bytes for Solidity ABI
    const IS_DYNAMIC: bool = false;

    fn encode(&self, buf: &mut impl BufMut, mut offset: usize) -> Result<usize, CodecError> {
        let bytes = if is_big_endian::<B>() {
            self.to_be_bytes::<32>()
        } else {
            self.to_le_bytes::<32>()
        };
        offset += write_aligned_slice::<B, ALIGN>(buf, &bytes);
        Ok(offset)
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        if buf.remaining() < offset + 32 {
            return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                expected: offset + 32,
                found: buf.remaining(),
                msg: "buf too small to read Uint".to_string(),
            }));
        }

        let chunk = &buf.chunk()[offset..offset + 32];
        let value_slice = &chunk[32 - Self::BYTES..];

        let value = if is_big_endian::<B>() {
            Self::from_be_slice(value_slice)
        } else {
            Self::from_le_slice(value_slice)
        };

        Ok(value)
    }

    fn partial_decode(_buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        Ok((offset, 32))
    }
}

impl<
        const BITS: usize,
        const LIMBS: usize,
        B: ByteOrder,
        const ALIGN: usize,
        const IS_STATIC: bool,
    > Encoder<B, ALIGN, false, IS_STATIC> for Signed<BITS, LIMBS>
{
    const HEADER_SIZE: usize = Self::BYTES;
    const IS_DYNAMIC: bool = false;

    fn encode(&self, buf: &mut impl BufMut, mut offset: usize) -> Result<usize, CodecError> {
        let bytes = if is_big_endian::<B>() {
            self.to_be_bytes::<32>()
        } else {
            self.to_le_bytes::<32>()
        };
        offset += write_aligned_slice::<B, ALIGN>(buf, &bytes);
        Ok(offset)
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let word_size = align_up::<ALIGN>(Self::BYTES);

        if buf.remaining() < offset + word_size {
            return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                expected: offset + word_size,
                found: buf.remaining(),
                msg: "buf too small to read Signed".to_string(),
            }));
        }

        let chunk = &buf.chunk()[offset..offset + word_size];
        let value_slice = &chunk[..Self::BYTES];

        let value = if is_big_endian::<B>() {
            Self::from_raw(Uint::<BITS, LIMBS>::from_be_slice(value_slice))
        } else {
            Self::from_raw(Uint::<BITS, LIMBS>::from_le_slice(value_slice))
        };

        Ok(value)
    }

    fn partial_decode(_buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        let word_size = align_up::<ALIGN>(Self::BYTES);
        Ok((offset, word_size))
    }
}

impl<
        const BITS: usize,
        const LIMBS: usize,
        B: ByteOrder,
        const ALIGN: usize,
        const IS_STATIC: bool,
    > Encoder<B, ALIGN, true, IS_STATIC> for Signed<BITS, LIMBS>
{
    const HEADER_SIZE: usize = 32; // Always 32 bytes for Solidity ABI
    const IS_DYNAMIC: bool = false;

    fn encode(&self, buf: &mut impl BufMut, offset: usize) -> Result<usize, CodecError> {
        todo!()
        // let slice = get_aligned_slice::<B, 32>(buf, offset, 32);
        //
        // let bytes = if is_big_endian::<B>() {
        //     self.into_raw().to_be_bytes_vec()
        // } else {
        //     self.into_raw().to_le_bytes_vec()
        // };
        //
        // // For Solidity ABI, right-align the data
        // slice[32 - Self::BYTES..].copy_from_slice(&bytes);
        //
        // // For signed integers, we need to sign-extend the value
        // // If the most significant bit of the value is set (negative number),
        // // fill the padding with 1s, otherwise fill with 0s
        // if self.is_negative() {
        //     slice[..32 - Self::BYTES].fill(0xFF);
        // } else {
        //     slice[..32 - Self::BYTES].fill(0);
        // }
        //
        // Ok(offset)
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        if buf.remaining() < offset + 32 {
            return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                expected: offset + 32,
                found: buf.remaining(),
                msg: "buf too small to read Signed".to_string(),
            }));
        }

        let chunk = &buf.chunk()[offset..offset + 32];
        let value_slice = &chunk[32 - Self::BYTES..];

        let value = if is_big_endian::<B>() {
            Self::from_raw(Uint::<BITS, LIMBS>::from_be_slice(value_slice))
        } else {
            Self::from_raw(Uint::<BITS, LIMBS>::from_le_slice(value_slice))
        };

        Ok(value)
    }

    fn partial_decode(_buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        Ok((offset, 32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        byteorder::{BE, LE},
        CompactABI,
        SolidityABI,
    };
    #[cfg(test)]
    use alloy_primitives::{Address, U256};
    use alloy_primitives::{I128, I256};
    use byteorder::{BigEndian, LittleEndian};
    use bytes::BytesMut;

    #[test]
    fn test_write_to_existing_buf() {
        let existing_data = &[
            0, 0, 0, 0, 0, 0, 0, 32, // offset of the 1st bytes
            0, 0, 0, 0, 0, 0, 0, 12, // length of the 1st bytes
            0, 0, 0, 0, 0, 0, 0, 0, //
            0, 0, 0, 0, 0, 0, 0, 0, //
            72, 101, 108, 108, 111, 44, 32, 87, 111, 114, 108, 100, // b"Hello, World"
        ];
        let mut buf = BytesMut::new();
        buf.extend_from_slice(existing_data);

        let original = Bytes::from_static(b"Hello, World");
        // Write the data to the buf
        let _result =
            write_bytes::<BigEndian, 8, false>(&mut buf, 16, &original, original.len() as u32);

        let expected = [
            0, 0, 0, 0, 0, 0, 0, 32, // offset of the 1st bytes
            0, 0, 0, 0, 0, 0, 0, 12, // length of the 1st bytes
            0, 0, 0, 0, 0, 0, 0, 44, // offset of the 2nd bytes
            0, 0, 0, 0, 0, 0, 0, 12, // length of the 2nd bytes
            72, 101, 108, 108, 111, 44, 32, 87, 111, 114, 108, 100, // b"Hello, World"
            72, 101, 108, 108, 111, 44, 32, 87, 111, 114, 108, 100, // b"Hello, World"
        ];

        assert_eq!(buf.to_vec(), expected);

        let mut encoded = buf.freeze();
        println!("Encoded Bytes: {:?}", encoded.to_vec());

        let decoded = read_bytes::<BigEndian, 8, false>(&mut encoded, 0).unwrap();

        println!("Decoded Bytes: {:?}", decoded.to_vec());
        assert_eq!(decoded.to_vec(), original.to_vec());
    }

    #[test]
    fn test_address_encode_decode() {
        let original = Address::from([0x42; 20]);
        let mut buf = BytesMut::new();

        <Address as Encoder<LittleEndian, 1, false, true>>::encode(&original, &mut buf, 0).unwrap();

        let encoded = buf.freeze();
        println!("Encoded Address: {}", hex::encode(&encoded));

        let decoded =
            <Address as Encoder<LittleEndian, 1, false, true>>::decode(&encoded, 0).unwrap();

        assert_eq!(original, decoded);
    }
    #[test]
    fn test_address_encode_decode_aligned() {
        let original = Address::from([0x42; 20]);
        let mut buf = BytesMut::new();

        <Address as Encoder<LittleEndian, 32, true, true>>::encode(&original, &mut buf, 0).unwrap();

        let encoded = buf.freeze();
        println!("Encoded Address: {}", hex::encode(&encoded));

        let decoded =
            <Address as Encoder<LittleEndian, 32, true, true>>::decode(&encoded, 0).unwrap();

        assert_eq!(original, decoded);
    }

    #[test]
    fn test_uint_encode_decode_le() {
        let original = U256::from(0x1234567890abcdef_u64);
        let mut buf = BytesMut::new();

        <U256 as Encoder<LittleEndian, 4, false, true>>::encode(&original, &mut buf, 0).unwrap();

        let encoded = buf.freeze();
        println!("Encoded U256 (LE): {}", hex::encode(&encoded));
        let expected_encoded = "efcdab9078563412000000000000000000000000000000000000000000000000";
        assert_eq!(hex::encode(&encoded), expected_encoded);

        let decoded = <U256 as Encoder<LittleEndian, 4, false, true>>::decode(&encoded, 0).unwrap();

        assert_eq!(original, decoded);
    }

    #[test]
    fn test_uint_encode_decode_be() {
        let original = U256::from(0x1234567890abcdef_u64);
        let mut buf = BytesMut::new();

        <U256 as Encoder<BigEndian, 4, false, true>>::encode(&original, &mut buf, 0).unwrap();

        let encoded = buf.freeze();
        println!("Encoded U256 (BE): {}", hex::encode(&encoded));
        let expected_encoded = "0000000000000000000000000000000000000000000000001234567890abcdef";
        assert_eq!(hex::encode(&encoded), expected_encoded);

        let decoded = <U256 as Encoder<BigEndian, 4, false, true>>::decode(&encoded, 0).unwrap();

        assert_eq!(original, decoded);
    }
    #[test]
    fn test_string_encoding_solidity() {
        let original = "Hello, World!!".to_string();
        let mut buf = BytesMut::new();
        <String as Encoder<BigEndian, 32, true, false>>::encode(&original, &mut buf, 0).unwrap();

        let encoded = buf.freeze();

        println!("Encoded String: {}", hex::encode(&encoded));

        let expected_encoded = "0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000e48656c6c6f2c20576f726c642121000000000000000000000000000000000000";

        assert_eq!(hex::encode(&encoded), expected_encoded);

        let decoded = <String as Encoder<BigEndian, 32, true, false>>::decode(&encoded, 0).unwrap();

        assert_eq!(original, decoded);
    }
    #[test]
    fn test_string_encoding_fluent() {
        let original = "Hello, World!!".to_string();
        let mut buf = BytesMut::new();
        <String as Encoder<LittleEndian, 4, false, false>>::encode(&original, &mut buf, 0).unwrap();

        let encoded = buf.freeze();

        println!("Encoded String: {}", hex::encode(&encoded));

        let expected_encoded = "080000000e00000048656c6c6f2c20576f726c6421210000";

        assert_eq!(hex::encode(&encoded), expected_encoded);

        let decoded =
            <String as Encoder<LittleEndian, 4, false, false>>::decode(&encoded, 0).unwrap();

        assert_eq!(original, decoded);
    }

    #[test]
    fn test_i128_solidity() {
        // Test cases with expected encodings
        let test_cases = [
            // Simple positive number
            (
                I128::try_from(42i32).unwrap(),
                "000000000000000000000000000000000000000000000000000000000000002a",
            ),
            // Simple negative number
            (
                I128::try_from(-42i32).unwrap(),
                "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffd6",
            ),
            // Zero
            (
                I128::try_from(0i32).unwrap(),
                "0000000000000000000000000000000000000000000000000000000000000000",
            ),
            // Negative one (-1)
            (
                I128::try_from(-1i32).unwrap(),
                "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            ),
            // Edge case: Maximum value (I128::MAX)
            (
                I128::MAX,
                "000000000000000000000000000000007fffffffffffffffffffffffffffffff",
            ),
            // Edge case: Minimum value (I128::MIN)
            (
                I128::MIN,
                "ffffffffffffffffffffffffffffffff80000000000000000000000000000000",
            ),
        ];

        // Test each case
        for (i, (test_value, expected_hex)) in test_cases.iter().enumerate() {
            println!(
                "Testing I128 encoding/decoding for case {}; {}",
                i, test_value
            );
            // Encode the value
            let mut buf = BytesMut::new();
            SolidityABI::encode(test_value, &mut buf).unwrap();
            let encoded = buf.freeze();

            // Verify encoding matches the expected value
            let expected_encoded = hex::decode(expected_hex).unwrap();
            assert_eq!(
                encoded.to_vec(),
                expected_encoded,
                "Case {}: I128 encoding doesn't match expected value",
                i
            );

            // Verify round-trip encoding/decoding
            let decoded = SolidityABI::<I128>::decode(&encoded, 0).unwrap();
            assert_eq!(
                decoded, *test_value,
                "Case {}: Round-trip encoding/decoding failed",
                i
            );
        }
    }

    #[test]
    fn test_i256_solidity() {
        // Test cases with expected encodings
        let test_cases = [
            // Simple positive number
            (
                I256::try_from(42i32).unwrap(),
                "000000000000000000000000000000000000000000000000000000000000002a",
            ),
            // Simple negative number
            (
                I256::try_from(-42i32).unwrap(),
                "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffd6",
            ),
            // Zero
            (
                I256::try_from(0i32).unwrap(),
                "0000000000000000000000000000000000000000000000000000000000000000",
            ),
            // Large positive number
            (
                I256::try_from(1234567890i64).unwrap(),
                "00000000000000000000000000000000000000000000000000000000499602d2",
            ),
            // Large negative number
            (
                I256::try_from(-1234567890i64).unwrap(),
                "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffb669fd2e",
            ),
            // Edge case: Maximum value (I256::MAX)
            (
                I256::MAX,
                "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            ),
            // Edge case: Minimum value (I256::MIN)
            (
                I256::MIN,
                "8000000000000000000000000000000000000000000000000000000000000000",
            ),
            // Edge case: Close to maximum (I256::MAX - 1)
            (
                I256::MAX - I256::ONE,
                "7ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe",
            ),
            // Edge case: Close to minimum (I256::MIN + 1)
            (
                I256::MIN + I256::ONE,
                "8000000000000000000000000000000000000000000000000000000000000001",
            ),
        ];

        // Test each case
        for (i, (test_value, expected_hex)) in test_cases.iter().enumerate() {
            println!(
                "Testing I256 encoding/decoding for case {}; {}",
                i, test_value
            );
            // Encode the value
            let mut buf = BytesMut::new();
            SolidityABI::encode(test_value, &mut buf).unwrap();
            let encoded = buf.freeze();

            // Verify encoding matches the expected value
            let expected_encoded = hex::decode(expected_hex).unwrap();
            assert_eq!(
                encoded.to_vec(),
                expected_encoded,
                "Case {}: I256 encoding doesn't match expected value",
                i
            );

            // Verify round-trip encoding/decoding
            let decoded = SolidityABI::<I256>::decode(&encoded, 0).unwrap();
            assert_eq!(
                decoded, *test_value,
                "Case {}: Round-trip encoding/decoding failed",
                i
            );
        }
    }

    #[test]
    fn test_i128_compact() {
        // Test cases with expected encodings
        // For CompactABI, I128 is encoded as 16 bytes in little-endian order
        let test_cases = [
            // Simple positive number
            (
                I128::try_from(42i32).unwrap(),
                "2a000000000000000000000000000000",
            ),
            // Simple negative number
            (
                I128::try_from(-42i32).unwrap(),
                "d6ffffffffffffffffffffffffffffff",
            ),
            // Zero
            (
                I128::try_from(0i32).unwrap(),
                "00000000000000000000000000000000",
            ),
            // Negative one (-1)
            (
                I128::try_from(-1i32).unwrap(),
                "ffffffffffffffffffffffffffffffff",
            ),
            // Edge case: Maximum value (I128::MAX)
            (I128::MAX, "ffffffffffffffffffffffffffffff7f"),
            // Edge case: Minimum value (I128::MIN)
            (I128::MIN, "00000000000000000000000000000080"),
        ];

        // Test each case
        for (i, (test_value, expected_hex)) in test_cases.iter().enumerate() {
            println!(
                "Testing I128 CompactABI encoding/decoding for case {}; {}",
                i, test_value
            );
            // Encode the value
            let mut buf = BytesMut::new();
            CompactABI::encode(test_value, &mut buf).unwrap();
            let encoded = buf.freeze();

            // Verify encoding matches the expected value
            let expected_encoded = hex::decode(expected_hex).unwrap();
            assert_eq!(
                encoded.to_vec(),
                expected_encoded,
                "Case {}: I128 CompactABI encoding doesn't match expected value",
                i
            );

            // Verify round-trip encoding/decoding
            let decoded = CompactABI::<I128>::decode(&encoded, 0).unwrap();
            assert_eq!(
                decoded, *test_value,
                "Case {}: CompactABI round-trip encoding/decoding failed",
                i
            );
        }
    }

    #[test]
    fn test_i256_compact() {
        // Test cases with expected encodings
        // For CompactABI, I256 uses little-endian byte order with 4-byte alignment
        let test_cases = [
            // Simple positive number
            (
                I256::try_from(42i32).unwrap(),
                "2a00000000000000000000000000000000000000000000000000000000000000",
            ),
            // Simple negative number
            (
                I256::try_from(-42i32).unwrap(),
                "d6ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            ),
            // Zero
            (
                I256::try_from(0i32).unwrap(),
                "0000000000000000000000000000000000000000000000000000000000000000",
            ),
            // Large positive number
            (
                I256::try_from(1234567890i64).unwrap(),
                "d202964900000000000000000000000000000000000000000000000000000000",
            ),
            // Large negative number
            (
                I256::try_from(-1234567890i64).unwrap(),
                "2efd69b6ffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            ),
            // Edge case: Maximum value (I256::MAX)
            (
                I256::MAX,
                "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
            ),
            // Edge case: Minimum value (I256::MIN)
            (
                I256::MIN,
                "0000000000000000000000000000000000000000000000000000000000000080",
            ),
        ];

        // Test each case
        for (i, (test_value, expected_hex)) in test_cases.iter().enumerate() {
            println!(
                "Testing I256 CompactABI encoding/decoding for case {}; {}",
                i, test_value
            );
            // Encode the value
            let mut buf = BytesMut::new();
            CompactABI::encode(test_value, &mut buf).unwrap();
            let encoded = buf.freeze();

            // Verify encoding matches expected value
            let expected_encoded = hex::decode(expected_hex).unwrap();
            assert_eq!(
                encoded.to_vec(),
                expected_encoded,
                "Case {}: I256 CompactABI encoding doesn't match expected value",
                i
            );

            // Verify round-trip encoding/decoding
            let decoded = CompactABI::<I256>::decode(&encoded, 0).unwrap();
            assert_eq!(
                decoded, *test_value,
                "Case {}: CompactABI round-trip encoding/decoding failed",
                i
            );
        }
    }

    #[test]
    fn test_i256_error_conditions() {
        // Test decoding from a buffer that's too small
        let too_small_buffer = BytesMut::new().freeze();
        let result = SolidityABI::<I256>::decode(&too_small_buffer, 0);
        assert!(result.is_err(), "Decoding from an empty buffer should fail");

        // Test decoding from a buffer that's smaller than required
        let small_buffer = BytesMut::from(&[0u8; 16][..]).freeze();
        let result = SolidityABI::<I256>::decode(&small_buffer, 0);
        assert!(
            result.is_err(),
            "Decoding from a buffer smaller than 32 bytes should fail"
        );

        // Test decoding with an offset that would cause reading beyond the buffer
        // (but not overflow)
        let buffer = BytesMut::from(&[0u8; 32][..]).freeze();
        let result = SolidityABI::<I256>::decode(&buffer, 1); // Just 1 byte offset is enough to cause an error
        assert!(
            result.is_err(),
            "Decoding with an offset that would read beyond buffer should fail"
        );

        // Test CompactABI decoding from a buffer that's too small
        let too_small_buffer = BytesMut::new().freeze();
        let result = CompactABI::<I256>::decode(&too_small_buffer, 0);
        assert!(
            result.is_err(),
            "CompactABI decoding from an empty buffer should fail"
        );
    }
}
