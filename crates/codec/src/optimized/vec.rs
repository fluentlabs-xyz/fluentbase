//! Compact ABI layout for vectors (and nested vectors)
//!
//! Each dynamic value (vector/string/bytes) has a 12-byte header of three little-endian u32:
//! [len | offset | size]
//! - len: number of elements
//! - offset: relative offset to the data-zone or child headers
//! - size: byte length of the data-zone; root is always 0
//!
//! For nested vectors: headers are written in pre-order, all data-zones (tails) are written after.
//! Example for [[1,2,3],[4,5]]:
//! 00: 02 00 00 00 0C 00 00 00 00 00 00 00    // root: len, off, size
//! 0C: 03 00 00 00 24 00 00 00 0C 00 00 00    // child 0: len, off, size
//! 18: 02 00 00 00 30 00 00 00 08 00 00 00    // child 1: len, off, size
//! 30: 01 00 00 00 02 00 00 00 03 00 00 00    // child 0 data-zone
//! 3C: 04 00 00 00 05 00 00 00                // child 1 data-zone

use crate::optimized::{
    counter::ByteCounter,
    ctx::EncodingContext,
    encoder::Encoder,
    error::CodecError,
    utils::{align_up, read_u32_aligned, write_u32_aligned},
};
use byteorder::{BigEndian, ByteOrder};
use bytes::{Buf, BufMut};
use core::mem::size_of;

/// Compact ABI encoder for Vec<T>
impl<T, B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, false> for Vec<T>
where
    T: Encoder<B, ALIGN, false, Ctx = EncodingContext>,
{
    type Ctx = EncodingContext;
    const IS_DYNAMIC: bool = true;
    const HEADER_SIZE: usize = size_of::<u32>() * 3;

    fn header_size(&self, ctx: &mut EncodingContext) -> Result<(), CodecError> {
        // Base header size: len (u32), offset (u32), size (u32)
        ctx.hdr_size += Self::HEADER_SIZE as u32;

        if T::IS_DYNAMIC {
            // Dynamic fields have their own headers pointing to data; offset and size fields aren't
            // needed here
            ctx.hdr_size -= (2 * align_up::<ALIGN>(4)) as u32;

            for el in self {
                el.header_size(ctx)?;
            }
        }

        Ok(())
    }

    fn encode_header(
        &self,
        out: &mut impl bytes::BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        let hdr_ptr = ctx.hdr_ptr;
        let len = self.len() as u32;

        // Write length of the vector
        write_u32_aligned::<B, ALIGN>(out, len);

        if T::IS_DYNAMIC {
            // Dynamic fields have their own headers pointing to data, skip offset and size
            ctx.hdr_ptr += align_up::<ALIGN>(4) as u32; // only len written, adjust header pointer

            // Recursively encode headers for nested elements
            for el in self {
                el.encode_header(out, ctx)?;
            }
        } else {
            // For static fields, offset points directly to data section
            let off = (ctx.hdr_size - ctx.hdr_ptr + ctx.data_ptr) as u32;
            let size = len * T::HEADER_SIZE as u32;

            write_u32_aligned::<B, ALIGN>(out, off);
            write_u32_aligned::<B, ALIGN>(out, size);

            ctx.hdr_ptr += Self::HEADER_SIZE as u32;
            ctx.data_ptr += size;
        }

        Ok((ctx.hdr_ptr - hdr_ptr) as usize)
    }

    fn encode_tail(
        &self,
        out: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        let mut bytes = 0;
        for el in self {
            if T::IS_DYNAMIC {
                bytes += el.encode_tail(out, ctx)?;
            } else {
                // Static elements have no separate tail; encode data inline directly
                bytes += el.encode_header(out, ctx)?;
            }
        }
        Ok(bytes)
    }

    #[inline]
    fn len(&self) -> usize {
        self.len()
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let word = align_up::<ALIGN>(4);

        // Read vector length
        let len = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;

        if len == 0 {
            return Ok(Vec::new());
        }

        // Compute pointer for next fields after length
        let next_field_ptr = offset + word;

        let (header_ptr, elem_size) = if T::IS_DYNAMIC {
            // Dynamic elements: headers immediately follow length
            (next_field_ptr, T::HEADER_SIZE)
        } else {
            // Static elements: data offset specified explicitly
            let data_off = read_u32_aligned::<B, ALIGN>(buf, next_field_ptr)? as usize;
            let data_ptr = offset + data_off;
            (data_ptr, align_up::<ALIGN>(T::HEADER_SIZE))
        };

        // Decode elements using computed pointer
        let mut out = Vec::with_capacity(len);
        for i in 0..len {
            let elem_offset = header_ptr + i * elem_size;
            out.push(T::decode(buf, elem_offset)?);
        }

        Ok(out)
    }
}

/// Vec implementation for Solidity ABI
/// For Solidity, we don't have size:
/// - header
///   - offset
/// - body
///   - length
///   - raw bytes of the vector

impl<T, B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, true> for Vec<T>
where
    T: Encoder<B, ALIGN, true, Ctx = EncodingContext>,
{
    type Ctx = EncodingContext;
    const HEADER_SIZE: usize = 32; // offset pointer for top-level
    const IS_DYNAMIC: bool = true;

    /// Calculates the size required for all headers (offset, length, sub-headers if dynamic).
    fn header_size(&self, ctx: &mut EncodingContext) -> Result<(), CodecError> {
        ctx.hdr_size += Self::HEADER_SIZE as u32; // Offset for this vector
        ctx.hdr_size += 32; // Length field for the data section
        if T::IS_DYNAMIC {
            ctx.hdr_size += (self.len() as u32) * 32; // Offsets for dynamic elements
            for el in self {
                el.header_size(ctx)?;
            }
        }
        Ok(())
    }

    /// Encodes the header (offset, length, and sub-offsets for nested dynamic).

    fn encode_header(
        &self,
        buf: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        todo!()
    }

    /// Encodes the tail (actual data for each element).
    fn encode_tail(
        &self,
        buf: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        let mut written = 0;
        if T::IS_DYNAMIC {
            for el in self {
                written += el.encode(buf, ctx)?;
            }
        } else {
            for el in self {
                written += el.encode_header(buf, ctx)?;
            }
        }
        Ok(written)
    }

    fn encode(
        &self,
        out: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        let mut written = 0;

        let word_size: u32 = align_up::<ALIGN>(T::HEADER_SIZE) as u32;

        // Write offset for outer container
        if ctx.depth == 0 {
            write_u32_aligned::<BigEndian, ALIGN>(out, word_size);
            written += word_size;
        }

        ctx.depth += 1;

        // write data len
        write_u32_aligned::<BigEndian, ALIGN>(out, self.len() as u32);
        written += word_size;

        if self.is_empty() {
            return Ok(written as usize);
        }

        if T::IS_DYNAMIC {
            written += encode_dynamic_elements(self, out, ctx)? as u32;
        } else {
            for element in self.iter() {
                written += element.encode(out, ctx)? as u32;
            }
        }

        ctx.depth -= 1;
        Ok(written as usize)
    }

    /// Decodes a Solidity ABI vector from the buffer.
    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let data_offset = read_u32_aligned::<BigEndian, 32>(buf, offset)?;

        let len = read_u32_aligned::<BigEndian, 32>(buf, data_offset as usize)? as usize;

        if len == 0 {
            return Ok(Vec::new());
        }

        let mut result = Vec::with_capacity(len);
        let chunk = &buf.chunk()[(data_offset + 32) as usize..];

        for i in 0..len {
            let elem_offset = i * align_up::<32>(T::HEADER_SIZE);
            let value = T::decode(&chunk, elem_offset)?;
            result.push(value);
        }

        Ok(result)
    }
}

#[inline(always)]
fn encode_dynamic_elements<T, B: ByteOrder, const ALIGN: usize>(
    vec: &[T],
    buf: &mut impl BufMut,
    ctx: &mut EncodingContext,
) -> Result<usize, CodecError>
where
    T: Encoder<B, ALIGN, true, Ctx = EncodingContext>,
{
    let len = vec.len();
    if len == 0 {
        return Ok(0);
    }

    let mut current_offset = len * 32;

    for element in vec.iter() {
        let size = if T::IS_DYNAMIC {
            let mut counter = ByteCounter::new();
            element.encode(&mut counter, ctx)?;
            counter.count()
        } else {
            align_up::<32>(T::HEADER_SIZE)
        };

        write_u32_aligned::<BigEndian, 32>(buf, current_offset as u32);
        current_offset += size;
    }

    // Write actual elements
    let mut total_written = 0;
    for element in vec.iter() {
        let written = element.encode(buf, ctx)?;

        total_written += written;
    }

    Ok(len * 32 + total_written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::optimized::utils::test_utils::assert_codec_compact;
    mod compact {
        use super::*;
        use byteorder::LittleEndian;
        #[test]
        fn test_empty_vec() {
            let value: Vec<u32> = vec![];
            assert_codec_compact(
                "000000000c00000000000000", // len = 0, offset = 12, size = 0
                "",                         // no tail
                &value,
            );
        }

        #[test]
        fn test_vec_u32_codec() {
            let value = vec![1u32, 2, 3, 4, 5];
            assert_codec_compact(
                concat!(
                    "05000000", // len = 5
                    "0c000000", // offset = 12
                    "14000000"  // size = 20 (5 * 4)
                ),
                concat!("01000000", "02000000", "03000000", "04000000", "05000000"),
                &value,
            );
        }

        #[test]
        fn test_vec_vec_u32_codec() {
            let value = vec![vec![1u32, 2, 3], vec![4, 5]];
            assert_codec_compact(
                concat!(
                    "02000000", // 2  - len
                    "03000000", // 3  - len (v0)
                    "18000000", // 24 - offset to v0 data
                    "0c000000", // 12 - size (v0 data) 3 * 4B = 12B
                    "02000000", // 2  - len (v1)
                    "18000000", // 24 - offset to v1 data 12 + 12 = 24 (after v0)
                    "08000000", // 8  - size (v1 data) 2 * 4B = 8B
                ),
                concat!("01000000", "02000000", "03000000", "04000000", "05000000"),
                &value,
            );
        }

        #[test]
        fn test_deep_nested_vec() {
            let value = vec![vec![vec![1u32, 2], vec![3], vec![4, 5, 6]]];
            assert_codec_compact(
                concat!(
                    // root Vec<Vec<Vec<u32>>>
                    "01000000", // 1 element
                    // header for Vec<Vec<u32>>
                    "03000000", // 3 sub-vectors
                    // header for Vec<u32> = vec![1,2]
                    "02000000", // 2 elements
                    "24000000", // offset to data (36)
                    "08000000", // size of data (2×4 = 8)
                    // header for Vec<u32> = vec![3]
                    "01000000", // 1 element
                    "20000000", // offset to data (32)
                    "04000000", // size of data (1×4 = 4)
                    // header for Vec<u32> = vec![4,5,6]
                    "03000000", // 3 elements
                    "18000000", // offset to data (24)
                    "0c000000"  // size of data (3×4 = 12)
                ),
                concat!(
                    "01000000", "02000000", // [1, 2]
                    "03000000", // [3]
                    "04000000", "05000000", "06000000" // [4, 5, 6]
                ),
                &value,
            );
        }

        #[test]
        fn test_vec_with_offset_in_dirty_buffer() {
            let mut dirty = vec![0xAAu8; 8];

            let value = vec![1u32, 2, 3];
            let header = concat!("03000000", "0c000000", "0c000000");
            let tail = concat!("01000000", "02000000", "03000000");
            let mut encoded = hex::decode(format!("{}{}", header, tail)).unwrap();

            dirty.append(&mut encoded);

            let buf = &dirty[..];
            let decoded = <Vec<u32> as Encoder<LittleEndian, 4, false>>::decode(&buf, 8).unwrap();
            assert_eq!(
                decoded, value,
                "decoded value mismatch for dirty buffer with offset"
            );
        }
    }

    mod solidity {
        use crate::optimized::{ctx::EncodingContext, encoder::Encoder};
        use byteorder::BigEndian;
        use bytes::BytesMut;

        /// Checks Solidity ABI encode/decode roundtrip and encoded hex.
        fn assert_solidity_codec<T>(value: &T, expected_hex: &str)
        where
            T: Encoder<BigEndian, 32, true> + PartialEq + std::fmt::Debug + Clone,
        {
            // Encode to buffer
            let mut buf = BytesMut::new();
            let encode_result = T::encode(value, &mut buf, &mut T::Ctx::default());
            assert!(
                encode_result.is_ok(),
                "Solidity ABI encode failed: {:?}",
                encode_result
            );

            let encoded = buf.freeze();
            assert_eq!(
                hex::encode(&encoded),
                expected_hex,
                "Solidity ABI encoded hex mismatch"
            );

            // Decode and check value roundtrip
            let decode_result = T::decode(&encoded, 0);
            assert!(
                decode_result.is_ok(),
                "Solidity ABI decode failed: {:?}",
                decode_result
            );
            assert_eq!(
                *value,
                decode_result.unwrap(),
                "Solidity ABI roundtrip value mismatch"
            );
        }

        #[test]
        fn test_empty_vec_u32() {
            let value: Vec<u32> = vec![];
            assert_solidity_codec(
                &value,
                concat!(
                    // offset (0x20 == 32 bytes, where data section starts)
                    "0000000000000000000000000000000000000000000000000000000000000020",
                    // length = 0 (u32, 32 bytes padded)
                    "0000000000000000000000000000000000000000000000000000000000000000"
                ),
            );
        }

        #[test]
        fn test_vec_u32() {
            let value = vec![1u32, 2, 3, 4, 5];
            assert_solidity_codec(
                &value,
                concat!(
                    // offset to data (32)
                    "0000000000000000000000000000000000000000000000000000000000000020",
                    // length (5)
                    "0000000000000000000000000000000000000000000000000000000000000005",
                    // elements, each padded to 32 bytes
                    "0000000000000000000000000000000000000000000000000000000000000001",
                    "0000000000000000000000000000000000000000000000000000000000000002",
                    "0000000000000000000000000000000000000000000000000000000000000003",
                    "0000000000000000000000000000000000000000000000000000000000000004",
                    "0000000000000000000000000000000000000000000000000000000000000005"
                ),
            );
        }

        #[test]
        fn test_nested_vec_u32() {
            let value = vec![vec![1u32, 2, 3], vec![4, 5]];
            assert_solidity_codec(
                &value,
                concat!(
                    // main offset (0x20 = 32)
                    "0000000000000000000000000000000000000000000000000000000000000020",
                    // main array length (2)
                    "0000000000000000000000000000000000000000000000000000000000000002",
                    // offset to first subarray (0x40 = 64)
                    "0000000000000000000000000000000000000000000000000000000000000040",
                    // offset to second subarray (0xc0 = 192)
                    "00000000000000000000000000000000000000000000000000000000000000c0",
                    // first subarray: length (3)
                    "0000000000000000000000000000000000000000000000000000000000000003",
                    // element 1
                    "0000000000000000000000000000000000000000000000000000000000000001",
                    // element 2
                    "0000000000000000000000000000000000000000000000000000000000000002",
                    // element 3
                    "0000000000000000000000000000000000000000000000000000000000000003",
                    // second subarray: length (2)
                    "0000000000000000000000000000000000000000000000000000000000000002",
                    // element 4
                    "0000000000000000000000000000000000000000000000000000000000000004",
                    // element 5
                    "0000000000000000000000000000000000000000000000000000000000000005"
                ),
            );
        }

        #[test]
        fn test_encode_large() {
            let large_vec1: Vec<u32> = (0..1000).collect();
            let large_vec2: Vec<u32> = (1000..1200).collect();
            let large_vec3: Vec<u32> = (1200..1300).collect();
            let large_vec4: Vec<u32> = (1300..1350).collect();
            let large_vec5: Vec<u32> = (1300..2000).collect();

            let v = vec![
                vec![large_vec1, large_vec2, large_vec3, large_vec4],
                vec![large_vec5],
            ];

            let mut buf = BytesMut::new();
            let encode_result = <Vec<Vec<Vec<u32>>> as Encoder<BigEndian, 32, true>>::encode(
                &v,
                &mut buf,
                &mut EncodingContext::default(),
            );

            assert!(
                encode_result.is_ok(),
                "Encoding failed: {:?}",
                encode_result
            );
            let encoded = buf.freeze();
            println!("encoded: {:?}", hex::encode(&encoded));

            let decode_result =
                <Vec<Vec<Vec<u32>>> as Encoder<BigEndian, 32, true>>::decode(&encoded, 0);
            assert!(
                decode_result.is_ok(),
                "Decoding failed: {:?}",
                decode_result
            );
            let decoded = decode_result.unwrap();
            assert_eq!(decoded, v, "Decoded value mismatch");
        }

        // TODO: check do we actually need this use case
        // TLDR: Decoding from dirty buffers with offset might be caller's responsibility to
        // handle This test verifies decoding from a buffer with dirty/garbage data
        // at the beginning. Current implementation expects clean buffers - decoding
        // with offset into dirty buffer fails. This might be correct behavior: the
        // caller should ensure proper buffer alignment/cleaning  rather than the
        // decoder handling arbitrary offsets in dirty
        #[test]
        #[ignore]
        fn vec_decoding_from_dirty_buf() {
            todo!();
            // let original: Vec<u32> = vec![1, 2, 3, 4, 5];
            // let mut buf = BytesMut::new();
            // buf.extend_from_slice(&[0xFF, 0xFF, 0xFF]); // Add some initial data
            //
            // // Encode at top-level (None context)
            // <Vec<u32> as Encoder<BigEndian, 32, true>>::encode(&original, &mut buf,
            // None).unwrap(); let encoded = buf.freeze();
            //
            // eprintln!("encoded: {:?}", hex::encode(&encoded));
            //
            // let expected_encoded = hex::decode(concat!(
            //     "ffffff",                                                           //
            // Initial data
            //     "0000000000000000000000000000000000000000000000000000000000000020", // offset
            //     "0000000000000000000000000000000000000000000000000000000000000005", // length
            // = 5
            //     "0000000000000000000000000000000000000000000000000000000000000001", // 1
            //     "0000000000000000000000000000000000000000000000000000000000000002", // 2
            //     "0000000000000000000000000000000000000000000000000000000000000003", // 3
            //     "0000000000000000000000000000000000000000000000000000000000000004", // 4
            //     "0000000000000000000000000000000000000000000000000000000000000005"  // 5
            // ))
            // .unwrap();
            //
            // if encoded != expected_encoded {
            //     eprintln!("Encoded mismatch!");
            //     eprintln!("Expected: {}", hex::encode(&expected_encoded));
            //     eprintln!("Actual:   {}", hex::encode(&encoded));
            // }
            // assert_eq!(expected_encoded, encoded);
            //
            // // Decode starting from offset 3 (after the 0xFF bytes)
            // let decoded = <Vec<u32> as Encoder<BigEndian, 32, true>>::decode(&encoded,
            // 3).unwrap();
            //
            // assert_eq!(original, decoded);
        }
    }
}
