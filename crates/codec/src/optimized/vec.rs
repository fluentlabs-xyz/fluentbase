use crate::optimized::{
    ctx::EncodingContext,
    encoder::Encoder,
    error::CodecError,
    utils::{align_up, read_u32_aligned, write_u32_aligned},
};
use byteorder::ByteOrder;
use bytes::{Buf, BufMut};
use core::mem::size_of;

/// Compact ABI layout for vectors (and nested vectors)
///
/// Each dynamic value (vector/string/bytes) has a 12-byte header of three little-endian u32:
/// [len | offset | size]
/// - len: number of elements
/// - offset: relative offset to the data-zone or child headers
/// - size: byte length of the data-zone; root is always 0
///
/// For nested vectors: headers are written in pre-order, all data-zones (tails) are written after.
impl<T, B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, false> for Vec<T>
where
    T: Encoder<B, ALIGN, false, Ctx = EncodingContext>,
{
    type Ctx = EncodingContext;
    const HEADER_SIZE: usize = size_of::<u32>() * 3;
    const IS_DYNAMIC: bool = true;

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

    #[inline]
    fn len(&self) -> usize {
        self.len()
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
    const HEADER_SIZE: usize = align_up::<ALIGN>(32); // offset pointer for top-level
    const IS_DYNAMIC: bool = true;

    /// Calculates the size required for all headers (offset, length, sub-headers if dynamic).
    fn header_size(&self, ctx: &mut EncodingContext) -> Result<(), CodecError> {
        ctx.hdr_size += Self::HEADER_SIZE as u32;
        Ok(())
    }

    /// Encodes the header (offset, length, and sub-offsets for nested dynamic).
    fn encode_header(
        &self,
        buf: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        // offset
        let offset = (ctx.hdr_size - ctx.hdr_ptr) + ctx.data_ptr;
        write_u32_aligned::<B, ALIGN>(buf, offset);
        ctx.hdr_ptr += <Self as Encoder<B, ALIGN, true>>::HEADER_SIZE as u32;

        // actual data size
        let mut temp_ctx = EncodingContext::new();
        let data_size = self.tail_size(&mut temp_ctx)?;
        ctx.data_ptr += data_size as u32;

        Ok(<Self as Encoder<B, ALIGN, true>>::HEADER_SIZE)
    }

    /// Encodes the tail (actual data for each element).
    fn encode_tail(
        &self,
        buf: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        let tail_start = buf.remaining_mut();

        // 1. Write the vector length (always 32 bytes in Solidity ABI)
        write_u32_aligned::<B, 32>(buf, self.len() as u32);

        if !T::IS_DYNAMIC {
            // For static elements (like u32), write them sequentially
            // Each element takes exactly 32 bytes in Solidity ABI

            for (i, element) in self.iter().enumerate() {
                let before = buf.remaining_mut();
                element.encode_header(buf, ctx)?;
                let written = before - buf.remaining_mut();
            }
        } else {
            // Dynamic elements require two-phase encoding:
            // Phase 1: Write all offsets
            // Phase 2: Write all data

            // Create a local context for this vector's internal structure
            // This isolates offset calculations from the parent context
            let mut local_ctx = EncodingContext::new();
            local_ctx.hdr_size = (self.len() * 32) as u32; // Space for all offsets
            local_ctx.hdr_ptr = 0;
            local_ctx.data_ptr = 0;

            // Phase 1: Calculate sizes and write offsets

            for (i, element) in self.iter().enumerate() {
                // Calculate where this element's data will start
                // Offset = space_for_all_offsets + accumulated_data_size
                let element_offset = local_ctx.hdr_size + local_ctx.data_ptr;

                // Write the offset (relative to the start of this vector's data)
                write_u32_aligned::<B, 32>(buf, element_offset);

                // Calculate how much space this element will need in the data section
                let mut temp_ctx = EncodingContext::new();
                let element_data_size = element.tail_size(&mut temp_ctx)?;

                // Update data pointer for next element
                local_ctx.data_ptr += element_data_size as u32;
            }

            // Phase 2: Write actual element data
            for (i, element) in self.iter().enumerate() {
                // For nested vectors, this will recursively call encode_tail
                element.encode_tail(buf, &mut local_ctx)?;
            }
        }

        let total_written = tail_start - buf.remaining_mut();

        Ok(total_written)
    }

    /// Decodes a Solidity ABI vector from the buffer.
    ///
    /// This implementation follows Solidity ABI conventions where all offsets are relative
    /// to the start of the encoded structure. The caller is responsible for ensuring the
    /// buffer contains valid ABI-encoded data at the specified offset.
    ///
    /// # Note
    ///
    /// This decoder does not handle "dirty" buffers with garbage data before the encoded
    /// content. For such cases, the caller must provide the correct starting offset where
    /// the valid ABI encoding begins.
    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let data_offset = read_u32_aligned::<B, ALIGN>(buf, offset)?;
        let len = read_u32_aligned::<B, ALIGN>(buf, data_offset as usize)? as usize;
        if len == 0 {
            return Ok(Vec::new());
        }

        let mut result = Vec::with_capacity(len);
        let chunk = &buf.chunk()[(data_offset + ALIGN as u32) as usize..];

        for i in 0..len {
            let elem_offset = i * align_up::<ALIGN>(T::HEADER_SIZE);
            let value = T::decode(&chunk, elem_offset)?;
            result.push(value);
        }

        Ok(result)
    }
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
        use crate::optimized::{
            ctx::EncodingContext,
            encoder::Encoder,
            utils::test_utils::{assert_codec_sol, encode_alloy_sol, print_encoded},
        };
        use alloy_primitives::hex::encode;
        use byteorder::BigEndian;
        use bytes::BytesMut;

        #[test]
        fn test_empty_vec_u32() {
            let value: Vec<u32> = vec![];
            assert_codec_sol(
                "0000000000000000000000000000000000000000000000000000000000000020",
                // length = 0 (u32, 32 bytes padded)
                "0000000000000000000000000000000000000000000000000000000000000000",
                &value,
            );
        }

        #[test]
        fn test_vec_u32() {
            let value = vec![1u32, 2, 3, 4, 5];
            assert_codec_sol(
                concat!(
                    // offset to data (32)
                    "0000000000000000000000000000000000000000000000000000000000000020",
                ),
                concat!(
                    // length (5)
                    "0000000000000000000000000000000000000000000000000000000000000005",
                    // elements, each padded to 32 bytes
                    "0000000000000000000000000000000000000000000000000000000000000001",
                    "0000000000000000000000000000000000000000000000000000000000000002",
                    "0000000000000000000000000000000000000000000000000000000000000003",
                    "0000000000000000000000000000000000000000000000000000000000000004",
                    "0000000000000000000000000000000000000000000000000000000000000005"
                ),
                &value,
            );
        }

        #[test]
        fn test_nested_vec_u32() {
            let value = vec![vec![1u32, 2, 3], vec![4, 5]];
            let expected_encoded = encode_alloy_sol(&value);
            print_encoded::<BigEndian, 32>(&expected_encoded);

            assert_codec_sol(
                concat!(
                    // main offset (0x20 = 32)
                    "0000000000000000000000000000000000000000000000000000000000000020",
                    // main array length (2)
                ),
                concat!(
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
                &value,
            );
        }

        #[test]
        fn test_deep_nested_u32() {
            let value = vec![vec![vec![1u32, 2], vec![3], vec![4, 5, 6]]];

            assert_codec_sol(
                concat!(
                    "0000000000000000000000000000000000000000000000000000000000000020", /* [0x0000] 0 = 32 */
                ),
                concat!(
                    "0000000000000000000000000000000000000000000000000000000000000001", /* [0x0020] 32 = 1 */
                    "0000000000000000000000000000000000000000000000000000000000000020", /* [0x0040] 64 = 32 */
                    "0000000000000000000000000000000000000000000000000000000000000003", /* [0x0060] 96 = 3 */
                    "0000000000000000000000000000000000000000000000000000000000000060", /* [0x0080] 128 = 96 */
                    "00000000000000000000000000000000000000000000000000000000000000c0", /* [0x00a0] 160 = 192 */
                    "0000000000000000000000000000000000000000000000000000000000000100", /* [0x00c0] 192 = 256 */
                    "0000000000000000000000000000000000000000000000000000000000000002", /* [0x00e0] 224 = 2 */
                    "0000000000000000000000000000000000000000000000000000000000000001", /* [0x0100] 256 = 1 */
                    "0000000000000000000000000000000000000000000000000000000000000002", /* [0x0120] 288 = 2 */
                    "0000000000000000000000000000000000000000000000000000000000000001", /* [0x0140] 320 = 1 */
                    "0000000000000000000000000000000000000000000000000000000000000003", /* [0x0160] 352 = 3 */
                    "0000000000000000000000000000000000000000000000000000000000000003", /* [0x0180] 384 = 3 */
                    "0000000000000000000000000000000000000000000000000000000000000004", /* [0x01a0] 416 = 4 */
                    "0000000000000000000000000000000000000000000000000000000000000005", /* [0x01c0] 448 = 5 */
                    "0000000000000000000000000000000000000000000000000000000000000006", /* [0x01e0] 480 = 6 */
                ),
                &value,
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

        // For solidity we
        #[ignore]
        #[test]
        fn vec_decoding_from_dirty_buf() {
            let original: Vec<u32> = vec![1, 2, 3, 4, 5];

            // Create context and calculate header size
            let mut ctx = EncodingContext::default();
            let _ = <Vec<u32> as Encoder<BigEndian, 32, true>>::header_size(&original, &mut ctx);

            // Create header buffer with garbage
            let mut header_buf = BytesMut::new();
            header_buf.extend_from_slice(&[0xFF, 0xFF, 0xFF]); // Add garbage

            // Encode header (offset)
            let w = <Vec<u32> as Encoder<BigEndian, 32, true>>::encode_header(
                &original,
                &mut header_buf,
                &mut ctx,
            );
            assert!(w.is_ok(), "encode_header failed: {:?}", w);

            // Expected header: garbage + offset
            let expected_header_hex = concat!(
                "ffffff",                                                           // garbage
                "0000000000000000000000000000000000000000000000000000000000000020"  // offset = 32
            );
            assert_eq!(
                expected_header_hex,
                encode(&header_buf),
                "header bytes mismatch"
            );

            // Encode tail (actual data)
            let mut tail_buf = BytesMut::new();
            let w = <Vec<u32> as Encoder<BigEndian, 32, true>>::encode_tail(
                &original,
                &mut tail_buf,
                &mut ctx,
            );
            assert!(w.is_ok(), "encode_tail failed: {:?}", w);

            // Expected tail: length + elements
            let expected_tail_hex = concat!(
                "0000000000000000000000000000000000000000000000000000000000000005", // length = 5
                "0000000000000000000000000000000000000000000000000000000000000001", // 1
                "0000000000000000000000000000000000000000000000000000000000000002", // 2
                "0000000000000000000000000000000000000000000000000000000000000003", // 3
                "0000000000000000000000000000000000000000000000000000000000000004", // 4
                "0000000000000000000000000000000000000000000000000000000000000005"  // 5
            );
            assert_eq!(expected_tail_hex, encode(&tail_buf), "tail bytes mismatch");

            // Combine header and tail
            let mut full_buf = header_buf.clone();
            full_buf.extend_from_slice(&tail_buf);

            // Decode from offset 3 (after garbage)
            let decoded = <Vec<u32> as Encoder<BigEndian, 32, true>>::decode(&full_buf, 3)
                .expect("decode failed");
            assert_eq!(decoded, original, "decoded value mismatch");
        }
    }
}
