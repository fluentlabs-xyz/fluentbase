use crate::{
    alloc::string::ToString,
    bytes_codec::{read_bytes, read_bytes_header, write_bytes},
    encoder::{align_up, get_aligned_slice, is_big_endian, write_u32_aligned, Encoder},
    error::{CodecError, DecodingError},
};
use alloc::string::String;
use alloy_primitives::{Address, Bytes, FixedBytes, Uint};
use byteorder::ByteOrder;
use bytes::{Buf, BytesMut};

impl<B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, true, false> for Bytes {
    const HEADER_SIZE: usize = 32;
    const IS_DYNAMIC: bool = true;

    /// Encode the bytes into the buffer for Solidity mode.
    /// First, we encode the header and write it to the given offset.
    /// After that, we encode the actual data and write it to the end of the buffer.
    fn encode(&self, buf: &mut BytesMut, offset: usize) -> Result<(), CodecError> {
        let aligned_header_size =
            align_up::<32>(<Self as Encoder<B, ALIGN, true, false>>::HEADER_SIZE);

        // Ensure the buffer has enough space for the offset + header size
        if buf.len() < offset + aligned_header_size {
            buf.resize(offset + aligned_header_size, 0);
        }

        // Write the offset of the data (current length of the buffer)
        write_u32_aligned::<B, ALIGN>(buf, offset, buf.len() as u32);

        // Write the actual data to the buffer at the current length
        let _ = write_bytes::<B, ALIGN, true>(buf, buf.len(), self, self.len() as u32);

        // Add padding if necessary to ensure the buffer remains aligned
        if buf.len() % ALIGN != 0 {
            let padding = ALIGN - (buf.len() % ALIGN);
            buf.resize(buf.len() + padding, 0);
        }

        Ok(())
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
    fn encode(&self, buf: &mut BytesMut, offset: usize) -> Result<(), CodecError> {
        let aligned_el_size = align_up::<ALIGN>(4);

        // Ensure the buffer has enough space for the offset and header size
        if buf.len() < offset + aligned_el_size {
            buf.resize(offset + aligned_el_size, 0);
        }

        let _ = write_bytes::<B, ALIGN, false>(buf, offset, self, self.len() as u32);

        // Add padding if necessary to ensure the buffer remains aligned
        if buf.len() % ALIGN != 0 {
            let padding = ALIGN - (buf.len() % ALIGN);
            buf.resize(buf.len() + padding, 0);
        }

        Ok(())
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

    fn encode(&self, buf: &mut BytesMut, offset: usize) -> Result<(), CodecError> {
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

    fn encode(&self, buf: &mut BytesMut, offset: usize) -> Result<(), CodecError> {
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
    fn encode(&self, buf: &mut BytesMut, offset: usize) -> Result<(), CodecError> {
        let slice = get_aligned_slice::<B, ALIGN>(buf, offset, N);
        slice.copy_from_slice(self.as_ref());
        Ok(())
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
    const HEADER_SIZE: usize = 32; // Always 32 bytes for Solidity ABI
    const IS_DYNAMIC: bool = false;

    /// Encode the fixed bytes into the buffer for Solidity mode.
    /// Writes the fixed bytes directly to the buffer at the given offset, zero-padding to 32 bytes.
    fn encode(&self, buf: &mut BytesMut, offset: usize) -> Result<(), CodecError> {
        let slice = get_aligned_slice::<B, 32>(buf, offset, 32);
        slice[..N].copy_from_slice(self.as_ref());
        // Zero-pad the rest
        slice[N..].fill(0);
        Ok(())
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
            fn encode(&self, buf: &mut BytesMut, offset: usize) -> Result<(), CodecError> {
                let slice = get_aligned_slice::<B, ALIGN>(buf, offset, <$type>::len_bytes());
                slice.copy_from_slice(self.as_ref());
                Ok(())
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
            fn encode(&self, buf: &mut BytesMut, offset: usize) -> Result<(), CodecError> {
                let slice = get_aligned_slice::<B, 32>(buf, offset, 32);
                let size = <$type>::len_bytes();
                // Zero-pad the beginning
                slice[..32 - size].fill(0);
                // Copy the address bytes to the end
                slice[32 - size..].copy_from_slice(self.as_ref());
                Ok(())
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

    fn encode(&self, buf: &mut BytesMut, offset: usize) -> Result<(), CodecError> {
        let word_size = align_up::<ALIGN>(Self::BYTES);

        let slice = get_aligned_slice::<B, ALIGN>(buf, offset, word_size);

        let bytes = if is_big_endian::<B>() {
            self.to_be_bytes_vec()
        } else {
            self.to_le_bytes_vec()
        };

        slice[..Self::BYTES].copy_from_slice(&bytes);

        Ok(())
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

    fn encode(&self, buf: &mut BytesMut, offset: usize) -> Result<(), CodecError> {
        let slice = get_aligned_slice::<B, 32>(buf, offset, 32);

        let bytes = if is_big_endian::<B>() {
            self.to_be_bytes_vec()
        } else {
            self.to_le_bytes_vec()
        };

        // For Solidity ABI, right-align the data
        slice[32 - Self::BYTES..].copy_from_slice(&bytes);
        slice[..32 - Self::BYTES].fill(0); // Zero-pad the rest

        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(test)]
    use alloy_primitives::{Address, U256};
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
}
