#![allow(warnings)]
use crate::{
    alloc::string::ToString,
    encoder::{align_up, get_aligned_indices, is_big_endian, Encoder},
    error::{CodecError, DecodingError},
    write_u32_aligned,
};
use byteorder::ByteOrder;
use bytes::{Buf, BufMut, BytesMut};
use core::{marker::PhantomData, mem::size_of};

impl<B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool, const IS_STATIC: bool>
    Encoder<B, ALIGN, SOL_MODE, IS_STATIC> for PhantomData<B>
{
    const HEADER_SIZE: usize = 0;
    const IS_DYNAMIC: bool = false;

    fn encode(&self, _buf: &mut impl BufMut, offset: usize) -> Result<usize, CodecError> {
        Ok(0)
    }

    fn decode(_buf: &impl Buf, _offset: usize) -> Result<Self, CodecError> {
        Ok(PhantomData)
    }

    fn partial_decode(_buf: &impl Buf, _offset: usize) -> Result<(usize, usize), CodecError> {
        Ok((0, 0))
    }
}

impl<B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool, const IS_STATIC: bool>
    Encoder<B, ALIGN, SOL_MODE, IS_STATIC> for u8
{
    const HEADER_SIZE: usize = size_of::<u8>();
    const IS_DYNAMIC: bool = false;

    fn encode(&self, buf: &mut impl BufMut, _offset: usize) -> Result<usize, CodecError> {
        let alignment = ALIGN.max(1);
        if is_big_endian::<B>() {
            buf.put_bytes(0, alignment - 1);
            buf.put_u8(*self);
        } else {
            buf.put_u8(*self);
            buf.put_bytes(0, alignment - 1);
        }
        Ok(alignment)
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let word_size =
            align_up::<ALIGN>(<Self as Encoder<B, ALIGN, SOL_MODE, IS_STATIC>>::HEADER_SIZE);
        if buf.remaining() < offset + word_size {
            return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                expected: offset + word_size,
                found: buf.remaining(),
                msg: "buf too small to read aligned u8".to_string(),
            }));
        }
        let chunk = &buf.chunk()[offset..];
        let value = if is_big_endian::<B>() {
            chunk[word_size - 1]
        } else {
            chunk[0]
        };
        Ok(value)
    }

    fn partial_decode(_buf: &impl Buf, _offset: usize) -> Result<(usize, usize), CodecError> {
        Ok((
            0,
            align_up::<ALIGN>(<Self as Encoder<B, ALIGN, SOL_MODE, IS_STATIC>>::HEADER_SIZE),
        ))
    }
}

impl<B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool, const IS_STATIC: bool>
    Encoder<B, ALIGN, SOL_MODE, IS_STATIC> for bool
{
    const HEADER_SIZE: usize = size_of::<bool>();
    const IS_DYNAMIC: bool = false;

    fn encode(&self, buf: &mut impl BufMut, mut offset: usize) -> Result<usize, CodecError> {
        let offset_before = offset;
        let alignment = ALIGN.saturating_sub(1);
        let value: u8 = if *self { 1 } else { 0 };
        if is_big_endian::<B>() {
            // For big-endian, copy to the end of the aligned array
            buf.put_bytes(0, alignment);
            offset += alignment;
            buf.put_u8(value);
            offset += 1;
        } else {
            // For little-endian, copy to the start of the aligned array
            buf.put_u8(value);
            offset += 1;
            buf.put_bytes(0, alignment);
            offset += alignment;
        }
        Ok(offset - offset_before)
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let value = <u8 as Encoder<B, ALIGN, SOL_MODE, true>>::decode(buf, offset)?;
        Ok(value != 0)
    }

    fn partial_decode(_buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        Ok((
            offset,
            <Self as Encoder<B, ALIGN, SOL_MODE, IS_STATIC>>::HEADER_SIZE,
        ))
    }
}

macro_rules! impl_int {
    ($typ:ty, $read_method:ident, $write_method_be:ident, $write_method_le:ident) => {
        impl<B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool, const IS_STATIC: bool>
            Encoder<B, ALIGN, SOL_MODE, IS_STATIC> for $typ
        {
            const HEADER_SIZE: usize = core::mem::size_of::<$typ>();
            const IS_DYNAMIC: bool = false;

            fn encode(&self, buf: &mut impl BufMut, offset: usize) -> Result<usize, CodecError> {
                let alignment = ALIGN.max(size_of::<$typ>());
                if is_big_endian::<B>() {
                    buf.put_bytes(0, alignment - size_of::<$typ>());
                    buf.$write_method_be(*self);
                } else {
                    buf.$write_method_le(*self);
                    buf.put_bytes(0, alignment - size_of::<$typ>());
                }
                Ok(alignment)
            }

            fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
                let word_size = align_up::<ALIGN>(
                    <Self as Encoder<B, ALIGN, SOL_MODE, IS_STATIC>>::HEADER_SIZE,
                );

                if buf.remaining() < offset + ALIGN {
                    return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                        expected: offset + ALIGN,
                        found: buf.remaining(),
                        msg: "buf too small to decode value".to_string(),
                    }));
                }

                let chunk = &buf.chunk()[offset..];
                let value = if is_big_endian::<B>() {
                    B::$read_method(
                        &chunk[word_size
                            - <Self as Encoder<B, ALIGN, SOL_MODE, IS_STATIC>>::HEADER_SIZE
                            ..word_size],
                    )
                } else {
                    B::$read_method(
                        &chunk[..<Self as Encoder<B, ALIGN, SOL_MODE, IS_STATIC>>::HEADER_SIZE],
                    )
                };

                Ok(value)
            }

            fn partial_decode(
                _buf: &impl Buf,
                offset: usize,
            ) -> Result<(usize, usize), CodecError> {
                Ok((
                    offset,
                    <Self as Encoder<B, ALIGN, SOL_MODE, IS_STATIC>>::HEADER_SIZE,
                ))
            }
        }
    };
}

impl_int!(u16, read_u16, put_u16, put_u16_le);
impl_int!(u32, read_u32, put_u32, put_u32_le);
impl_int!(u64, read_u64, put_u64, put_u64_le);
impl_int!(i16, read_i16, put_i16, put_i16_le);
impl_int!(i32, read_i32, put_i32, put_i32_le);
impl_int!(i64, read_i64, put_i64, put_i64_le);

/// Encodes and decodes Option<T> where T is an Encoder.
/// The encoded data is prefixed with a single byte that indicates whether the Option is Some or
/// None. Single byte will be aligned to ALIGN.
impl<T, B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool, const IS_STATIC: bool>
    Encoder<B, ALIGN, SOL_MODE, IS_STATIC> for Option<T>
where
    T: Sized + Encoder<B, ALIGN, SOL_MODE, true> + Default,
{
    const HEADER_SIZE: usize = 1 + T::HEADER_SIZE;
    const IS_DYNAMIC: bool = false;

    fn encode(&self, buf: &mut impl BufMut, offset: usize) -> Result<usize, CodecError> {
        match self {
            Some(inner_value) => {
                Encoder::<B, ALIGN, SOL_MODE, IS_STATIC>::encode(&1u32, buf, offset)?;
                inner_value.encode(buf, offset + ALIGN)
            }
            None => {
                Encoder::<B, ALIGN, SOL_MODE, IS_STATIC>::encode(&0u32, buf, offset)?;
                let default_value = T::default();
                default_value.encode(buf, offset + ALIGN)
            }
        }
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let aligned_header =
            align_up::<ALIGN>(<Self as Encoder<B, ALIGN, SOL_MODE, IS_STATIC>>::HEADER_SIZE);

        if buf.remaining() < offset + aligned_header {
            return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                expected: offset + aligned_header,
                found: buf.remaining(),
                msg: "buf too small".to_string(),
            }));
        }

        let chunk = &buf.chunk()[offset..];
        let option_flag = if is_big_endian::<B>() {
            chunk[aligned_header - 1]
        } else {
            chunk[0]
        };

        let chunk = &buf.chunk()[offset + ALIGN..];

        if option_flag != 0 {
            let inner_value = T::decode(&chunk, 0)?;
            Ok(Some(inner_value))
        } else {
            Ok(None)
        }
    }

    fn partial_decode(buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        let aligned_header =
            align_up::<ALIGN>(<Self as Encoder<B, ALIGN, SOL_MODE, IS_STATIC>>::HEADER_SIZE);

        if buf.remaining() < offset + aligned_header {
            return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                expected: offset + aligned_header,
                found: buf.remaining(),
                msg: "buf too small".to_string(),
            }));
        }

        let chunk = &buf.chunk()[offset..];
        let option_flag = if is_big_endian::<B>() {
            chunk[ALIGN - 1]
        } else {
            chunk[0]
        };

        let chunk = &buf.chunk()[offset + ALIGN..];

        if option_flag != 0 {
            let (_, inner_size) = T::partial_decode(&chunk, 0)?;
            Ok((offset, aligned_header + inner_size))
        } else {
            let aligned_data_size = align_up::<ALIGN>(T::HEADER_SIZE);
            Ok((offset, aligned_header + aligned_data_size))
        }
    }
}

impl<
        T,
        B: ByteOrder,
        const ALIGN: usize,
        const SOL_MODE: bool,
        const N: usize,
        const IS_STATIC: bool,
    > Encoder<B, ALIGN, SOL_MODE, IS_STATIC> for [T; N]
where
    T: Sized + Encoder<B, ALIGN, SOL_MODE, IS_STATIC> + Default + Copy,
{
    const HEADER_SIZE: usize = align_up::<ALIGN>(T::HEADER_SIZE) * N;
    const IS_DYNAMIC: bool = false;

    fn encode(&self, buf: &mut impl BufMut, mut offset: usize) -> Result<usize, CodecError> {
        let offset_before = offset;
        for item in self.iter() {
            offset += item.encode(buf, offset)?;
        }
        Ok(offset - offset_before)
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let item_size = align_up::<ALIGN>(T::HEADER_SIZE);
        let total_size = offset + (item_size * N);

        if buf.remaining() < total_size {
            return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                expected: total_size,
                found: buf.remaining(),
                msg: "buf too small to decode [T; N]".to_string(),
            }));
        }

        let mut result = [T::default(); N];

        for (i, item) in result.iter_mut().enumerate() {
            *item = T::decode(buf, offset + (item_size * i))?;
        }

        Ok(result)
    }

    fn partial_decode(_buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        let item_size = align_up::<ALIGN>(T::HEADER_SIZE);
        let total_size = item_size * N;

        Ok((offset, total_size))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SolidityPackedABI;
    use byteorder::{BigEndian, LittleEndian};
    use bytes::{Bytes, BytesMut};

    #[test]
    fn test_u8_be_encode_decode() {
        let original: u8 = 1;
        const ALIGNMENT: usize = 32;

        let mut buf = BytesMut::new();

        println!("Buffer capacity: {}", buf.capacity());

        let encoding_result =
            <u8 as Encoder<BigEndian, { ALIGNMENT }, false, true>>::encode(&original, &mut buf, 0);

        assert!(encoding_result.is_ok());

        let expected_encoded = "0000000000000000000000000000000000000000000000000000000000000001";

        assert_eq!(hex::encode(&buf), expected_encoded);

        let buf_for_decode = buf.clone().freeze();
        let decoded =
            <u8 as Encoder<BigEndian, { ALIGNMENT }, false, true>>::decode(&buf_for_decode, 0)
                .unwrap();

        assert_eq!(original, decoded);
        println!("encoded: {:?}", buf);

        let partial_decoded =
            <u8 as Encoder<BigEndian, { ALIGNMENT }, false, true>>::partial_decode(
                &buf.clone().freeze(),
                0,
            )
            .unwrap();
        assert_eq!(partial_decoded, (0, ALIGNMENT));
    }

    #[test]
    fn test_u8_le_encode_decode() {
        let original: u8 = 1;
        const ALIGNMENT: usize = 32;
        let mut buf = BytesMut::new();

        println!("Buffer capacity: {}", buf.capacity());

        let encoding_result = <u8 as Encoder<LittleEndian, { ALIGNMENT }, false, true>>::encode(
            &original, &mut buf, 0,
        );

        assert!(encoding_result.is_ok());

        let expected_encoded = "0100000000000000000000000000000000000000000000000000000000000000";

        let encoded = buf.freeze();
        println!("Encoded: {:?}", encoded);
        assert_eq!(hex::encode(&encoded), expected_encoded);

        let decoded =
            <u8 as Encoder<LittleEndian, { ALIGNMENT }, false, true>>::decode(&encoded, 0).unwrap();
        println!("Decoded: {}", decoded);

        assert_eq!(original, decoded);

        let partial_decoded =
            <u8 as Encoder<LittleEndian, { ALIGNMENT }, false, true>>::partial_decode(&encoded, 0)
                .unwrap();

        assert_eq!(partial_decoded, (0, 32));
    }

    #[test]
    fn test_bool_be_encode_decode() {
        let original: bool = true;
        const ALIGNMENT: usize = 32;

        let mut buf = BytesMut::new();

        println!("Buffer capacity: {}", buf.capacity());

        let encoding_result = <bool as Encoder<BigEndian, { ALIGNMENT }, false, true>>::encode(
            &original, &mut buf, 0,
        );

        assert!(encoding_result.is_ok());

        let expected_encoded = "0000000000000000000000000000000000000000000000000000000000000001";

        assert_eq!(hex::encode(&buf), expected_encoded);

        let buf_for_decode = buf.clone().freeze();
        let decoded =
            <bool as Encoder<BigEndian, { ALIGNMENT }, false, true>>::decode(&buf_for_decode, 0)
                .unwrap();

        assert_eq!(original, decoded);
        println!("encoded: {:?}", buf);

        let partial_decoded =
            <bool as Encoder<BigEndian, { ALIGNMENT }, false, true>>::partial_decode(
                &buf.clone().freeze(),
                0,
            )
            .unwrap();
        assert_eq!(partial_decoded, (0, 1));
    }

    #[test]
    fn test_bool_le_encode_decode() {
        let original: bool = true;
        const ALIGNMENT: usize = 32;

        let mut buf = BytesMut::new();

        println!("Buffer capacity: {}", buf.capacity());

        let encoding_result = <bool as Encoder<LittleEndian, { ALIGNMENT }, false, true>>::encode(
            &original, &mut buf, 0,
        );

        assert!(encoding_result.is_ok());

        let expected_encoded = "0100000000000000000000000000000000000000000000000000000000000000";

        assert_eq!(hex::encode(&buf), expected_encoded);

        let buf_for_decode = buf.clone().freeze();
        let decoded =
            <bool as Encoder<LittleEndian, { ALIGNMENT }, false, true>>::decode(&buf_for_decode, 0)
                .unwrap();

        assert_eq!(original, decoded);
        println!("encoded: {:?}", buf);

        let partial_decoded =
            <bool as Encoder<LittleEndian, { ALIGNMENT }, false, true>>::partial_decode(
                &buf.clone().freeze(),
                0,
            )
            .unwrap();
        assert_eq!(partial_decoded, (0, 1));
    }

    #[test]
    fn test_u32_encode_decode_le() {
        let original: u32 = 0x12345678;
        let mut buf = BytesMut::new();

        <u32 as Encoder<LittleEndian, 8, false, true>>::encode(&original, &mut buf, 0).unwrap();

        println!("Encoded: {:?}", buf);

        assert_eq!(buf.to_vec(), vec![0x78, 0x56, 0x34, 0x12, 0, 0, 0, 0]);

        let buf_for_decode = buf.freeze();
        let decoded =
            <u32 as Encoder<LittleEndian, 8, false, true>>::decode(&buf_for_decode, 0).unwrap();

        assert_eq!(original, decoded);
    }

    #[test]
    fn test_u32_encode_decode_be() {
        let original: u32 = 0x12345678;
        let mut buf = BytesMut::new();

        <u32 as Encoder<BigEndian, 8, false, true>>::encode(&original, &mut buf, 0).unwrap();

        let encoded = buf.freeze();
        println!("{:?}", hex::encode(&encoded));
        assert_eq!(
            &encoded,
            &vec![0x00, 0x00, 0x00, 0x00, 0x12, 0x34, 0x56, 0x78]
        );

        let decoded = <u32 as Encoder<BigEndian, 8, false, true>>::decode(&encoded, 0).unwrap();
        println!("Decoded: {}", decoded);

        assert_eq!(original, decoded);
    }

    #[test]
    fn test_i64_encode_decode_be() {
        let original: i64 = 0x1234567890ABCDEF;
        let mut buf = BytesMut::new();

        <i64 as Encoder<BigEndian, 8, false, true>>::encode(&original, &mut buf, 0).unwrap();

        let encoded = buf.freeze();
        println!("Encoded: {:?}", hex::encode(&encoded));
        assert_eq!(
            &encoded,
            &vec![0x12, 0x34, 0x56, 0x78, 0x90, 0xAB, 0xCD, 0xEF]
        );

        let decoded = <i64 as Encoder<BigEndian, 8, false, true>>::decode(&encoded, 0).unwrap();
        println!("Decoded: {}", decoded);

        assert_eq!(original, decoded);
    }
    #[test]
    fn test_u32_wasm_abi_encode_decode() {
        let original: u32 = 0x12345678;
        let mut buf = BytesMut::new();

        // Encode
        <u32 as Encoder<LittleEndian, 4, false, true>>::encode(&original, &mut buf, 0).unwrap();

        // Check encoded format
        assert_eq!(buf.to_vec(), vec![0x78, 0x56, 0x34, 0x12]);

        // Decode
        let decoded = <u32 as Encoder<LittleEndian, 4, false, true>>::decode(&buf, 0).unwrap();

        // Check decoded value
        assert_eq!(original, decoded);
    }

    #[test]
    fn test_u32_solidity_abi_encode_decode() {
        let original: u32 = 0x12345678;
        let mut buf = BytesMut::new();

        // Encode
        <u32 as Encoder<BigEndian, 32, true, true>>::encode(&original, &mut buf, 0).unwrap();

        // Check encoded format (32 bytes, right-aligned)
        let expected = vec![
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0x12, 0x34, 0x56, 0x78,
        ];
        assert_eq!(buf.to_vec(), expected);

        // Decode
        let decoded = <u32 as Encoder<BigEndian, 32, true, true>>::decode(&buf, 0).unwrap();

        // Check decoded value
        assert_eq!(original, decoded);
    }

    #[test]
    fn test_option_u32_encode_decode() {
        let original: Option<u32> = Some(0x12345678);
        let mut buf = BytesMut::new();

        let ok =
            <Option<u32> as Encoder<LittleEndian, 4, false, true>>::encode(&original, &mut buf, 0);
        assert!(ok.is_ok());

        let encoded = buf.freeze();
        println!("Encoded: {:?}", &encoded.to_vec());
        assert_eq!(
            encoded,
            Bytes::from_static(&[0x01, 0x00, 0x00, 0x00, 0x78, 0x56, 0x34, 0x12])
        );

        let decoded = <Option<u32> as Encoder<LittleEndian, 4, false, true>>::decode(&encoded, 0);

        assert_eq!(original, decoded.unwrap());
    }

    #[test]
    fn test_u8_array_encode_decode_le_with_alignment() {
        let original: [u8; 5] = [1, 2, 3, 4, 5];
        let mut buf = BytesMut::new();

        <[u8; 5] as Encoder<LittleEndian, 4, false, true>>::encode(&original, &mut buf, 0).unwrap();

        let encoded = buf.freeze();
        println!("Encoded: {:?}", hex::encode(&encoded));

        // Check that the encoded data is correct and properly aligned
        assert_eq!(
            &encoded.to_vec(),
            &[
                0x01, 0x00, 0x00, 0x00, // First byte aligned to 4 bytes
                0x02, 0x00, 0x00, 0x00, // Second byte aligned to 4 bytes
                0x03, 0x00, 0x00, 0x00, // Third byte aligned to 4 bytes
                0x04, 0x00, 0x00, 0x00, // Fourth byte aligned to 4 bytes
                0x05, 0x00, 0x00, 0x00 // Fifth byte aligned to 4 bytes
            ]
        );

        println!("Encoded: {:?}", encoded.to_vec());
        println!("encoded len: {}", encoded.len());
        let decoded =
            <[u8; 5] as Encoder<LittleEndian, 4, false, true>>::decode(&encoded, 0).unwrap();
        println!("Decoded: {:?}", decoded);

        assert_eq!(original, decoded);
    }

    #[test]
    fn test_packed_encoding() {
        let value1: u32 = 0x12345678;
        let value2: u16 = 0x9ABC;
        let value3: u8 = 0xDE;
        let mut buf = BytesMut::new();

        SolidityPackedABI::<u32>::encode(&value1, &mut buf).unwrap();
        SolidityPackedABI::<u16>::encode(&value2, &mut buf).unwrap();
        SolidityPackedABI::<u8>::encode(&value3, &mut buf).unwrap();

        assert_eq!(buf.to_vec(), vec![0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE]);
    }

    #[test]
    fn test_packed_array() {
        let arr: [u16; 3] = [0x1234, 0x5678, 0x9ABC];
        let mut buf = BytesMut::new();

        // Using the existing implementation with packed parameters
        SolidityPackedABI::<[u16; 3]>::encode(&arr, &mut buf).unwrap();

        assert_eq!(buf.to_vec(), vec![0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC]);

        let decoded = SolidityPackedABI::<[u16; 3]>::decode(&buf, 0).unwrap();
        assert_eq!(arr, decoded);
    }
}
