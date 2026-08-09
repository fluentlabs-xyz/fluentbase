use crate::{
    alloc::string::ToString,
    encoder::{
        align_up, checked_decode_slice, checked_decode_slice_from, get_aligned_indices,
        get_aligned_slice, is_big_endian, read_u32_aligned, validate_collection_body,
        write_u32_aligned, Encoder,
    },
    error::{CodecError, DecodingError},
};
use alloc::vec::Vec;
use byteorder::ByteOrder;
use bytes::{Buf, BytesMut};
use core::{marker::PhantomData, mem::size_of};

impl<B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool, const IS_STATIC: bool>
    Encoder<B, ALIGN, SOL_MODE, IS_STATIC> for PhantomData<B>
{
    const HEADER_SIZE: usize = 0;
    const IS_DYNAMIC: bool = false;

    fn encode(&self, _buf: &mut BytesMut, _offset: usize) -> Result<(), CodecError> {
        Ok(())
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

    fn encode(&self, buf: &mut BytesMut, offset: usize) -> Result<(), CodecError> {
        let word_size =
            align_up::<ALIGN>(<Self as Encoder<B, ALIGN, SOL_MODE, IS_STATIC>>::HEADER_SIZE);

        if buf.len() < offset + word_size {
            buf.resize(offset + word_size, 0);
        }

        let write_to = get_aligned_slice::<B, ALIGN>(buf, offset, 1);

        write_to[0] = *self;

        Ok(())
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let word_size =
            align_up::<ALIGN>(<Self as Encoder<B, ALIGN, SOL_MODE, IS_STATIC>>::HEADER_SIZE);

        let chunk =
            checked_decode_slice(buf, offset, word_size, "buf too small to read aligned u8")?;

        let value = if is_big_endian::<B>() {
            chunk[word_size - 1]
        } else {
            chunk[0]
        };

        Ok(value)
    }

    fn partial_decode(_buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        Ok((
            offset,
            align_up::<ALIGN>(<Self as Encoder<B, ALIGN, SOL_MODE, IS_STATIC>>::HEADER_SIZE),
        ))
    }
}

impl<B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool, const IS_STATIC: bool>
    Encoder<B, ALIGN, SOL_MODE, IS_STATIC> for bool
{
    const HEADER_SIZE: usize = core::mem::size_of::<bool>();
    const IS_DYNAMIC: bool = false;

    fn encode(&self, buf: &mut BytesMut, offset: usize) -> Result<(), CodecError> {
        let value: u8 = if *self { 1 } else { 0 };

        <u8 as Encoder<B, ALIGN, SOL_MODE, true>>::encode(&value, buf, offset)
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let value = <u8 as Encoder<B, ALIGN, SOL_MODE, true>>::decode(buf, offset)?;

        Ok(value != 0)
    }

    fn partial_decode(_buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        Ok((
            offset,
            align_up::<ALIGN>(<Self as Encoder<B, ALIGN, SOL_MODE, IS_STATIC>>::HEADER_SIZE),
        ))
    }
}

macro_rules! impl_int {
    ($typ:ty, $read_method:ident, $write_method:ident) => {
        impl<B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool, const IS_STATIC: bool>
            Encoder<B, ALIGN, SOL_MODE, IS_STATIC> for $typ
        {
            const HEADER_SIZE: usize = core::mem::size_of::<$typ>();
            const IS_DYNAMIC: bool = false;

            fn encode(&self, buf: &mut BytesMut, offset: usize) -> Result<(), CodecError> {
                let word_size = align_up::<ALIGN>(
                    <Self as Encoder<B, ALIGN, SOL_MODE, IS_STATIC>>::HEADER_SIZE,
                );

                if buf.len() < offset + word_size {
                    buf.resize(offset + word_size, 0);
                }

                let (start, end) = get_aligned_indices::<B, ALIGN>(
                    offset,
                    <Self as Encoder<B, ALIGN, SOL_MODE, IS_STATIC>>::HEADER_SIZE,
                );

                B::$write_method(&mut buf[start..end], *self);

                // Sign-extend negative values; everything else, zero included, pads with zeros.
                // `< 0` is always false for the unsigned instantiations of this macro.
                #[allow(unused_comparisons)]
                let fill_val = if *self < 0 { 0xFF } else { 0x00 };

                for i in offset..start {
                    buf[i] = fill_val;
                }

                B::$write_method(&mut buf[start..end], *self);

                for i in end..(offset + word_size) {
                    buf[i] = fill_val;
                }

                Ok(())
            }

            fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
                let word_size = align_up::<ALIGN>(
                    <Self as Encoder<B, ALIGN, SOL_MODE, IS_STATIC>>::HEADER_SIZE,
                );

                // The read below spans the whole aligned word, so the buffer has to be checked
                // against `word_size` and not `ALIGN`: types wider than the alignment (`u64` and
                // `i64` with `ALIGN == 4`) otherwise pass the guard while truncated and panic
                // inside the byteorder read.
                let chunk =
                    checked_decode_slice(buf, offset, word_size, "buf too small to decode value")?;

                let value = if is_big_endian::<B>() {
                    B::$read_method(
                        &chunk[word_size
                            - <Self as Encoder<B, ALIGN, SOL_MODE, IS_STATIC>>::HEADER_SIZE..],
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
                // The aligned width, not the value's own size: this reports the extent of the
                // field in the buffer, which is what `Address`, `FixedBytes` and `Uint` report and
                // what `Option<T>` adds up. `HEADER_SIZE` alone makes `Some` and `None` disagree.
                Ok((
                    offset,
                    align_up::<ALIGN>(
                        <Self as Encoder<B, ALIGN, SOL_MODE, IS_STATIC>>::HEADER_SIZE,
                    ),
                ))
            }
        }
    };
}

impl_int!(u16, read_u16, write_u16);
impl_int!(u32, read_u32, write_u32);
impl_int!(u64, read_u64, write_u64);
impl_int!(i16, read_i16, write_i16);
impl_int!(i32, read_i32, write_i32);
impl_int!(i64, read_i64, write_i64);

/// Encodes and decodes `Option<T>` where `T` is an encoder.
/// The encoded data is prefixed with a single byte that indicates whether the Option is Some or
/// None. Single byte will be aligned to ALIGN.
impl<T, B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool, const IS_STATIC: bool>
    Encoder<B, ALIGN, SOL_MODE, IS_STATIC> for Option<T>
where
    T: Sized + Encoder<B, ALIGN, SOL_MODE, true> + Default,
{
    const HEADER_SIZE: usize = 1 + T::HEADER_SIZE;
    const IS_DYNAMIC: bool = false;

    fn encode(&self, buf: &mut BytesMut, offset: usize) -> Result<(), CodecError> {
        let aligned_header =
            align_up::<ALIGN>(<Self as Encoder<B, ALIGN, SOL_MODE, IS_STATIC>>::HEADER_SIZE);
        if buf.len() < offset + aligned_header {
            buf.resize(offset + aligned_header, 0);
        }
        // Get aligned slice for the option flag
        let flag_slice = get_aligned_slice::<B, ALIGN>(buf, offset, 1);
        flag_slice[0] = if self.is_some() { 1 } else { 0 };

        let inner_offset = offset + ALIGN;

        match self {
            Some(inner_value) => inner_value.encode(buf, inner_offset)?,
            None => {
                let default_value = T::default();
                default_value.encode(buf, inner_offset)?;
            }
        };

        Ok(())
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

impl<T, B: ByteOrder, const ALIGN: usize, const N: usize, const IS_STATIC: bool>
    Encoder<B, ALIGN, false, IS_STATIC> for [T; N]
where
    T: Sized + Encoder<B, ALIGN, false, IS_STATIC> + Default + Copy,
{
    const HEADER_SIZE: usize = align_up::<ALIGN>(T::HEADER_SIZE) * N;
    const IS_DYNAMIC: bool = false;

    fn encode(&self, buf: &mut BytesMut, offset: usize) -> Result<(), CodecError> {
        let item_size = align_up::<ALIGN>(T::HEADER_SIZE);

        if buf.len() < offset + (item_size * N) {
            buf.resize(offset + (item_size * N), 0);
        }

        for (i, item) in self.iter().enumerate() {
            item.encode(buf, offset + (item_size * i))?;
        }

        Ok(())
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

/// Collects `N` decoded elements into an array without requiring `T: Copy`.
///
/// `[T::default(); N]` would, and that bound is what used to make `[String; N]` - a legal Solidity
/// `string[N]` - impossible to encode at all.
fn collect_array<T, const N: usize>(items: Vec<T>) -> Result<[T; N], CodecError> {
    items.try_into().map_err(|_| {
        CodecError::Decoding(DecodingError::InvalidData(
            "decoded element count does not match the array length".to_string(),
        ))
    })
}

/// Solidity `T[N]`.
///
/// Two things about this impl are not obvious.
///
/// The elements are always encoded at word alignment, whatever this array's own `ALIGN` is: the
/// spec says array elements are padded even in packed mode ("array elements are padded, but still
/// encoded in-place"), while a value passed directly is not. `ALIGN` therefore governs where the
/// array as a whole sits, and the element encoding is pinned to 32. At `ALIGN = 32` the two
/// coincide and the bytes are unchanged.
///
/// And `T[k]` is dynamic exactly when `T` is - `string[3]` is a dynamic type. In that case the
/// array occupies one offset word in its parent's head area and its body is `k` offset words
/// followed by the elements' tails, the same layout `Vec<T>` uses minus the length prefix.
impl<T, B: ByteOrder, const ALIGN: usize, const N: usize, const IS_STATIC: bool>
    Encoder<B, ALIGN, true, IS_STATIC> for [T; N]
where
    T: Sized + Encoder<B, 32, true, IS_STATIC>,
{
    const HEADER_SIZE: usize = if T::IS_DYNAMIC {
        32
    } else {
        align_up::<32>(T::HEADER_SIZE) * N
    };
    const IS_DYNAMIC: bool = T::IS_DYNAMIC;

    fn encode(&self, buf: &mut BytesMut, offset: usize) -> Result<(), CodecError> {
        if T::IS_DYNAMIC {
            // One offset word in the head; the body goes at the end of the buffer. Elements encode
            // into their own buffer so the offsets they write are relative to the start of the
            // array's body, which is what the spec requires.
            if buf.len() < offset + 32 {
                buf.resize(offset + 32, 0);
            }
            write_u32_aligned::<B, ALIGN>(buf, offset, buf.len() as u32);

            let head_width = align_up::<32>(4);
            let mut body = BytesMut::zeroed(head_width * N);
            for (i, item) in self.iter().enumerate() {
                item.encode(&mut body, head_width * i)?;
            }
            buf.extend_from_slice(&body);

            return Ok(());
        }

        let item_size = align_up::<32>(T::HEADER_SIZE);
        let end = offset + (item_size * N);

        if buf.len() < end {
            buf.resize(end, 0);
        }

        if offset.is_multiple_of(32) {
            for (i, item) in self.iter().enumerate() {
                item.encode(buf, offset + (item_size * i))?;
            }
            return Ok(());
        }

        // Packed mode can place an array at an offset that is not a whole number of words, and the
        // element encoder rounds whatever offset it is handed up to a word. Encode each element on
        // its own, where that rounding is a no-op, and copy it into place.
        let mut element = BytesMut::new();
        for (i, item) in self.iter().enumerate() {
            element.clear();
            element.resize(item_size, 0);
            item.encode(&mut element, 0)?;
            element.resize(item_size, 0);
            let at = offset + (item_size * i);
            buf[at..at + item_size].copy_from_slice(&element);
        }

        Ok(())
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let mut items = Vec::new();
        items.try_reserve(N).map_err(|_| {
            CodecError::Decoding(DecodingError::InvalidData(
                "unable to reserve array capacity".to_string(),
            ))
        })?;

        if T::IS_DYNAMIC {
            let body_offset = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;
            let body = checked_decode_slice_from(buf, body_offset, "array body exceeds input")?;
            let head_width = align_up::<32>(4);
            validate_collection_body(N, head_width, body.len())?;

            for i in 0..N {
                items.push(T::decode(&body, head_width * i)?);
            }

            return collect_array(items);
        }

        let item_size = align_up::<32>(T::HEADER_SIZE);
        let total_size = offset + (item_size * N);

        if buf.remaining() < total_size {
            return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                expected: total_size,
                found: buf.remaining(),
                msg: "buf too small to decode [T; N]".to_string(),
            }));
        }

        if offset.is_multiple_of(32) {
            for i in 0..N {
                items.push(T::decode(buf, offset + (item_size * i))?);
            }

            return collect_array(items);
        }

        // Mirrors the unaligned branch of `encode`.
        let chunk = buf.chunk();
        for i in 0..N {
            let at = offset + (item_size * i);
            let element = &chunk[at..at + item_size];
            items.push(T::decode(&element, 0)?);
        }

        collect_array(items)
    }

    fn partial_decode(buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        if T::IS_DYNAMIC {
            let body_offset = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;
            return Ok((body_offset, align_up::<32>(4) * N));
        }

        let item_size = align_up::<32>(T::HEADER_SIZE);
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

        let mut buf = BytesMut::zeroed(ALIGNMENT);

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
        let mut buf = BytesMut::zeroed(ALIGNMENT);

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

        let mut buf = BytesMut::zeroed(ALIGNMENT);

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
        assert_eq!(partial_decoded, (0, ALIGNMENT));
    }

    #[test]
    fn test_bool_le_encode_decode() {
        let original: bool = true;
        const ALIGNMENT: usize = 32;

        let mut buf = BytesMut::zeroed(ALIGNMENT);

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
        assert_eq!(partial_decoded, (0, ALIGNMENT));
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
        let mut buf = BytesMut::with_capacity(8);

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

        SolidityPackedABI::<u32>::encode(&value1, &mut buf, 0).unwrap();
        SolidityPackedABI::<u16>::encode(&value2, &mut buf, 4).unwrap();
        SolidityPackedABI::<u8>::encode(&value3, &mut buf, 6).unwrap();

        assert_eq!(buf.to_vec(), vec![0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE]);
    }

    #[test]
    fn test_packed_array() {
        let arr: [u16; 3] = [0x1234, 0x5678, 0x9ABC];
        let mut buf = BytesMut::new();

        // Using the existing implementation with packed parameters
        SolidityPackedABI::<[u16; 3]>::encode(&arr, &mut buf, 0).unwrap();

        // "array elements are padded, but still encoded in-place" - only the direct arguments of
        // `abi.encodePacked` go in unpadded, so each `uint16` here still occupies a whole word.
        let mut expected = vec![0u8; 96];
        expected[30..32].copy_from_slice(&[0x12, 0x34]);
        expected[62..64].copy_from_slice(&[0x56, 0x78]);
        expected[94..96].copy_from_slice(&[0x9A, 0xBC]);
        assert_eq!(buf.to_vec(), expected);

        let decoded = SolidityPackedABI::<[u16; 3]>::decode(&buf, 0).unwrap();
        assert_eq!(arr, decoded);
    }

    /// Encodes `value` with `ALIGN == 4` and asserts that every truncation of the resulting word
    /// is rejected with a decoding error instead of panicking, in both byte orders.
    macro_rules! assert_truncated_decode_errors {
        ($typ:ty, $value:expr) => {{
            const ALIGN: usize = 4;
            let value: $typ = $value;

            let mut le_buf = BytesMut::new();
            <$typ as Encoder<LittleEndian, ALIGN, false, true>>::encode(&value, &mut le_buf, 0)
                .unwrap();
            let mut be_buf = BytesMut::new();
            <$typ as Encoder<BigEndian, ALIGN, false, true>>::encode(&value, &mut be_buf, 0)
                .unwrap();

            let word_size = le_buf.len();
            assert_eq!(word_size, align_up::<ALIGN>(size_of::<$typ>()));
            assert_eq!(be_buf.len(), word_size);

            // A full word decodes back to the original value.
            let le_full = le_buf.clone().freeze();
            assert_eq!(
                <$typ as Encoder<LittleEndian, ALIGN, false, true>>::decode(&le_full, 0).unwrap(),
                value
            );
            let be_full = be_buf.clone().freeze();
            assert_eq!(
                <$typ as Encoder<BigEndian, ALIGN, false, true>>::decode(&be_full, 0).unwrap(),
                value
            );

            // Every short buffer is an error, including the 4..7 byte range that used to slip
            // past the `ALIGN`-sized guard and panic for the 8-byte types.
            for len in 0..word_size {
                let le_short = Bytes::copy_from_slice(&le_buf[..len]);
                assert!(
                    <$typ as Encoder<LittleEndian, ALIGN, false, true>>::decode(&le_short, 0)
                        .is_err(),
                    "LE decode of {} bytes should fail for {}",
                    len,
                    stringify!($typ)
                );

                let be_short = Bytes::copy_from_slice(&be_buf[..len]);
                assert!(
                    <$typ as Encoder<BigEndian, ALIGN, false, true>>::decode(&be_short, 0).is_err(),
                    "BE decode of {} bytes should fail for {}",
                    len,
                    stringify!($typ)
                );
            }
        }};
    }

    #[test]
    fn test_truncated_native_widths_do_not_panic() {
        assert_truncated_decode_errors!(u8, 0xAB);
        assert_truncated_decode_errors!(u16, 0xABCD);
        assert_truncated_decode_errors!(u32, 0xABCDEF01);
        assert_truncated_decode_errors!(u64, 0x0123456789ABCDEF);
        assert_truncated_decode_errors!(i16, -0x1234);
        assert_truncated_decode_errors!(i32, -0x12345678);
        assert_truncated_decode_errors!(i64, -0x123456789ABCDEF);
    }

    #[test]
    fn test_truncated_decode_reports_full_word() {
        // A 4-byte buffer satisfies `ALIGN` but not the 8-byte `u64` word.
        let short = Bytes::from_static(&[0xFF, 0xFF, 0xFF, 0xFF]);
        let err = <u64 as Encoder<LittleEndian, 4, false, true>>::decode(&short, 0).unwrap_err();

        match err {
            CodecError::Decoding(DecodingError::BufferTooSmall {
                expected, found, ..
            }) => {
                assert_eq!(expected, 8);
                assert_eq!(found, 4);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn test_decode_offset_overflow_is_rejected() {
        let buf = Bytes::from_static(&[0u8; 8]);
        assert!(<u64 as Encoder<LittleEndian, 4, false, true>>::decode(&buf, usize::MAX).is_err());
    }

    /// Encodes three `u64`s back to back at `ALIGN == 4` and checks that each one round-trips at
    /// its own offset, and that a buffer missing the last byte errors instead of panicking.
    macro_rules! assert_multi_value_u64_vector {
        ($byte_order:ty) => {{
            const ALIGN: usize = 4;
            let values: [u64; 3] = [0, 0x0123456789ABCDEF, u64::MAX];

            let mut buf = BytesMut::new();
            for (i, value) in values.iter().enumerate() {
                <u64 as Encoder<$byte_order, ALIGN, false, true>>::encode(value, &mut buf, i * 8)
                    .unwrap();
            }
            assert_eq!(buf.len(), 24);
            let encoded = buf.freeze();

            for (i, expected) in values.iter().enumerate() {
                let decoded =
                    <u64 as Encoder<$byte_order, ALIGN, false, true>>::decode(&encoded, i * 8)
                        .unwrap();
                assert_eq!(decoded, *expected, "element {} round-trip", i);
            }

            let truncated = encoded.slice(..23);
            assert!(
                <u64 as Encoder<$byte_order, ALIGN, false, true>>::decode(&truncated, 16).is_err()
            );
        }};
    }

    #[test]
    fn test_aligned_multi_value_u64_vector_decodes() {
        assert_multi_value_u64_vector!(LittleEndian);
        assert_multi_value_u64_vector!(BigEndian);
    }

    /// Encodes `value` at both alignments and byte orders and checks every padding byte.
    ///
    /// Padding is sign extension, so it is `0xFF` only for negative values. Zero used to take
    /// the negative branch and pad with `0xFF`, producing a word no other ABI implementation
    /// accepts - and the round-trip through this codec hid it, because decoding ignores the
    /// padding entirely.
    macro_rules! assert_padding {
        ($typ:ty, $value:expr, $expected_fill:expr) => {{
            fn check<T, B: ByteOrder, const ALIGN: usize>(value: &T, expected_fill: u8)
            where
                T: Encoder<B, ALIGN, true, true> + core::fmt::Debug,
            {
                let mut buf = BytesMut::new();
                value.encode(&mut buf, 0).unwrap();

                let value_width = size_of::<T>();
                let padding = if is_big_endian::<B>() {
                    0..buf.len() - value_width
                } else {
                    value_width..buf.len()
                };

                assert!(
                    buf[padding.clone()].iter().all(|byte| *byte == expected_fill),
                    "{value:?}: padding {padding:?} should be all 0x{expected_fill:02x}, got {}",
                    hex_words(&buf)
                );
            }

            let value: $typ = $value;
            check::<$typ, BigEndian, 32>(&value, $expected_fill);
            check::<$typ, LittleEndian, 4>(&value, $expected_fill);
        }};
    }

    fn hex_words(bytes: &[u8]) -> alloc::string::String {
        use alloc::string::String;
        bytes.iter().fold(String::new(), |mut acc, byte| {
            acc.push_str(&alloc::format!("{byte:02x}"));
            acc
        })
    }

    /// Zero is not negative, so its padding must be zeros in every width and byte order.
    #[test]
    fn test_zero_pads_with_zeros_not_sign_extension() {
        assert_padding!(u16, 0, 0x00);
        assert_padding!(u32, 0, 0x00);
        assert_padding!(u64, 0, 0x00);
        assert_padding!(i16, 0, 0x00);
        assert_padding!(i32, 0, 0x00);
        assert_padding!(i64, 0, 0x00);
    }

    /// The neighbours of zero keep the behaviour the fix must not change.
    #[test]
    fn test_padding_follows_sign_around_zero() {
        assert_padding!(u16, 1, 0x00);
        assert_padding!(u32, 1, 0x00);
        assert_padding!(u64, u64::MAX, 0x00);
        assert_padding!(i16, 1, 0x00);
        assert_padding!(i16, -1, 0xFF);
        assert_padding!(i32, -1, 0xFF);
        assert_padding!(i32, i32::MIN, 0xFF);
        assert_padding!(i64, -1, 0xFF);
        assert_padding!(i64, i64::MAX, 0x00);
    }

    /// The whole point of the padding: a zero must survive a round-trip through the codec and
    /// still be the canonical zero word on the wire.
    #[test]
    fn test_zero_encodes_to_the_canonical_word() {
        let mut buf = BytesMut::new();
        <u64 as Encoder<BigEndian, 32, true, true>>::encode(&0, &mut buf, 0).unwrap();
        assert_eq!(buf.len(), 32);
        assert_eq!(hex_words(&buf), "0".repeat(64));

        let encoded = buf.freeze();
        assert_eq!(
            <u64 as Encoder<BigEndian, 32, true, true>>::decode(&encoded, 0).unwrap(),
            0
        );
    }
}
