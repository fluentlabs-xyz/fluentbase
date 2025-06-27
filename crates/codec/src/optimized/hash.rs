use crate::optimized::{
    ctx::EncodingContext,
    encoder::Encoder,
    error::CodecError,
    utils::{align_up, read_u32_aligned, write_u32_aligned},
};
use alloc::vec::Vec;
use byteorder::ByteOrder;
use bytes::{Buf, BufMut};
use core::{fmt::Debug, hash::Hash, mem::size_of};
use hashbrown::{HashMap, HashSet};

/// Compact ABI encoder for HashMap<K, V>
impl<K, V, B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, false> for HashMap<K, V>
where
    K: Encoder<B, ALIGN, false, Ctx = EncodingContext> + Ord + Eq + Hash + Debug,
    V: Encoder<B, ALIGN, false, Ctx = EncodingContext> + Debug,
{
    type Ctx = EncodingContext;
    const HEADER_SIZE: usize = size_of::<u32>() * 5; // length (4) + keys_header (8) + values_header (8)
    const IS_DYNAMIC: bool = true; // length

    fn header_size(&self, ctx: &mut EncodingContext) -> Result<(), CodecError> {
        // Base header: len (u32), keys offset+size, values offset+size
        ctx.hdr_size += Self::HEADER_SIZE as u32;

        if V::IS_DYNAMIC {
            // For dynamic values, offset and size are stored in nested headers
            // So we don't need offset and size fields here (remove 8 bytes)
            ctx.hdr_size -= (2 * align_up::<ALIGN>(4)) as u32;

            // Keys always static

            // Calculate nested header sizes for values
            for value in self.values() {
                value.header_size(ctx)?;
            }
        }

        Ok(())
    }
    fn encode_header(
        &self,
        out: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        let hdr_ptr_start = ctx.hdr_ptr;
        let len = self.len() as u32;

        // Write length of hashmap
        write_u32_aligned::<B, ALIGN>(out, len);
        ctx.hdr_ptr += align_up::<ALIGN>(4) as u32;

        let mut entries: Vec<_> = self.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));

        // Write keys offset and size (always static)
        let keys_off = (ctx.hdr_size - hdr_ptr_start + ctx.data_ptr) as u32;
        let keys_size = len * K::HEADER_SIZE as u32;
        write_u32_aligned::<B, ALIGN>(out, keys_off);
        write_u32_aligned::<B, ALIGN>(out, keys_size);
        ctx.hdr_ptr += align_up::<ALIGN>(8) as u32;
        ctx.data_ptr += keys_size;

        if V::IS_DYNAMIC {
            // No values offset/size written
            ctx.depth += 1;
            for (_, value) in &entries {
                value.encode_header(out, ctx)?;
            }
            ctx.depth -= 1;
        } else {
            // Write values offset and size (static case)
            let values_off = (ctx.hdr_size - hdr_ptr_start + ctx.data_ptr) as u32;
            let values_size = len * V::HEADER_SIZE as u32;
            write_u32_aligned::<B, ALIGN>(out, values_off);
            write_u32_aligned::<B, ALIGN>(out, values_size);
            ctx.hdr_ptr += align_up::<ALIGN>(8) as u32;
            ctx.data_ptr += values_size;
        }

        Ok((ctx.hdr_ptr - hdr_ptr_start) as usize)
    }

    fn encode_tail(
        &self,
        out: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        let mut bytes = 0;

        // Sort entries to maintain consistent ordering
        let mut entries: Vec<_> = self.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));

        // First, encode keys
        for (key, _) in &entries {
            if K::IS_DYNAMIC {
                bytes += key.encode_tail(out, ctx)?;
            } else {
                bytes += key.encode_header(out, ctx)?;
            }
        }

        // Then, encode values
        for (_, value) in &entries {
            if V::IS_DYNAMIC {
                bytes += value.encode_tail(out, ctx)?;
            } else {
                bytes += value.encode_header(out, ctx)?;
            }
        }

        Ok(bytes)
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let aligned_u32 = align_up::<ALIGN>(4);

        // Read hashmap length
        let len = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;
        if len == 0 {
            return Ok(HashMap::new());
        }

        let mut hdr_ptr = offset + aligned_u32;

        // Decode keys offset and size
        let keys_offset = read_u32_aligned::<B, ALIGN>(buf, hdr_ptr)? as usize;
        hdr_ptr += aligned_u32;
        let keys_size = read_u32_aligned::<B, ALIGN>(buf, hdr_ptr)? as usize;
        hdr_ptr += aligned_u32;

        let keys_ptr = offset + keys_offset;

        if buf.remaining() < keys_ptr + keys_size {
            return Err(CodecError::BufferTooSmallMsg {
                expected: keys_ptr + keys_size,
                actual: buf.remaining(),
                message: "Not enough data for keys",
            });
        }

        // Special handling for dynamic nested values:
        // If values are dynamic, headers for nested values follow keys immediately.
        let (values_ptr, elem_hdr_size) = if V::IS_DYNAMIC {
            (hdr_ptr, V::HEADER_SIZE)
        } else {
            // Static values: offset and size fields explicitly provided
            let values_offset = read_u32_aligned::<B, ALIGN>(buf, hdr_ptr)? as usize;
            hdr_ptr += aligned_u32;
            let values_size = read_u32_aligned::<B, ALIGN>(buf, hdr_ptr)? as usize;
            (offset + values_offset, align_up::<ALIGN>(V::HEADER_SIZE))
        };

        // Decode keys from calculated positions
        let mut keys = Vec::with_capacity(len);
        for i in 0..len {
            let key_offset = keys_ptr + i * align_up::<ALIGN>(K::HEADER_SIZE);
            keys.push(K::decode(buf, key_offset)?);
        }

        // Decode values, handling dynamic nesting implicitly via recursive decode calls
        let mut values = Vec::with_capacity(len);
        for i in 0..len {
            let value_offset = values_ptr + i * elem_hdr_size;
            values.push(V::decode(buf, value_offset)?);
        }

        // Combine keys and values into final hashmap
        Ok(keys.into_iter().zip(values.into_iter()).collect())
    }

    #[inline]
    fn len(&self) -> usize {
        self.len()
    }
}

impl<T, B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, false> for HashSet<T>
where
    T: Encoder<B, ALIGN, false, Ctx = EncodingContext> + Ord + Eq + Hash + Debug,
{
    type Ctx = EncodingContext;
    const HEADER_SIZE: usize = size_of::<u32>() * 3; // len (4) + keys_offset (4) + keys_size (4)
    const IS_DYNAMIC: bool = true;

    fn header_size(&self, ctx: &mut EncodingContext) -> Result<(), CodecError> {
        ctx.hdr_size += Self::HEADER_SIZE as u32;

        if T::IS_DYNAMIC {
            // For dynamic keys, offset and size are stored in nested headers
            // So offset/size fields are not needed here
            ctx.hdr_size -= (2 * align_up::<ALIGN>(4)) as u32;

            for key in self {
                key.header_size(ctx)?;
            }
        }

        Ok(())
    }

    fn encode_header(
        &self,
        out: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        let hdr_ptr_start = ctx.hdr_ptr;
        let len = self.len() as u32;

        // Write length
        write_u32_aligned::<B, ALIGN>(out, len);
        ctx.hdr_ptr += align_up::<ALIGN>(4) as u32;

        // Sort elements to maintain consistent order
        let mut elements: Vec<_> = self.iter().collect();
        elements.sort();

        if T::IS_DYNAMIC {
            ctx.depth += 1;
            for key in elements {
                key.encode_header(out, ctx)?;
            }
            ctx.depth -= 1;
        } else {
            // Write keys offset and size (static case)
            let keys_off = (ctx.hdr_size - hdr_ptr_start + ctx.data_ptr) as u32;
            let keys_size = len * T::HEADER_SIZE as u32;
            write_u32_aligned::<B, ALIGN>(out, keys_off);
            write_u32_aligned::<B, ALIGN>(out, keys_size);
            ctx.hdr_ptr += align_up::<ALIGN>(8) as u32;
            ctx.data_ptr += keys_size;
        }

        Ok((ctx.hdr_ptr - hdr_ptr_start) as usize)
    }

    fn encode_tail(
        &self,
        out: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        let mut bytes = 0;

        // Sort elements to ensure consistent ordering
        let mut elements: Vec<_> = self.iter().collect();
        elements.sort();

        for element in elements {
            if T::IS_DYNAMIC {
                bytes += element.encode_tail(out, ctx)?;
            } else {
                bytes += element.encode_header(out, ctx)?;
            }
        }

        Ok(bytes)
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let aligned_u32 = align_up::<ALIGN>(4);

        // Read HashSet length
        let len = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;
        if len == 0 {
            return Ok(HashSet::new());
        }

        let hdr_ptr = offset + aligned_u32;

        // Special handling if keys are dynamic
        let (keys_ptr, elem_hdr_size) = if T::IS_DYNAMIC {
            // Dynamic keys: headers immediately follow length
            (hdr_ptr, T::HEADER_SIZE)
        } else {
            // Static keys: offset and size fields explicitly provided
            let keys_offset = read_u32_aligned::<B, ALIGN>(buf, hdr_ptr)? as usize;
            let keys_size = read_u32_aligned::<B, ALIGN>(buf, hdr_ptr + aligned_u32)? as usize;

            let keys_ptr = offset + keys_offset;

            if buf.remaining() < keys_ptr + keys_size {
                return Err(CodecError::BufferTooSmallMsg {
                    expected: keys_ptr + keys_size,
                    actual: buf.remaining(),
                    message: "Not enough data for HashSet elements",
                });
            }

            (keys_ptr, align_up::<ALIGN>(T::HEADER_SIZE))
        };

        // Decode elements
        let mut elements = HashSet::with_capacity(len);
        for i in 0..len {
            let key_offset = keys_ptr + i * elem_hdr_size;
            elements.insert(T::decode(buf, key_offset)?);
        }

        Ok(elements)
    }

    #[inline]
    fn len(&self) -> usize {
        self.len()
    }
}

#[cfg(test)]
mod tests {
    mod compact {
        use crate::optimized::{ctx::EncodingContext, encoder::Encoder, utils::read_u32_aligned};
        use byteorder::LittleEndian;
        use bytes::BytesMut;
        use core::hash::Hash;
        use hashbrown::HashMap;
        use hex::encode;
        fn assert_codec<T>(expected_header_hex: &str, expected_tail_hex: &str, value: &T)
        where
            T: Encoder<LittleEndian, 4, false, Ctx = EncodingContext> + PartialEq + std::fmt::Debug,
        {
            let mut ctx = EncodingContext::default();
            value.header_size(&mut ctx).unwrap();

            let mut header_buf = BytesMut::new();
            let w = value.encode_header(&mut header_buf, &mut ctx);
            assert!(w.is_ok(), "encode_header failed: {:?}", w);
            assert_eq!(
                expected_header_hex,
                encode(&header_buf),
                "header bytes mismatch"
            );

            let mut tail_buf = BytesMut::new();
            let w = value.encode_tail(&mut tail_buf, &mut ctx);
            assert!(w.is_ok(), "encode_tail failed: {:?}", w);
            assert_eq!(expected_tail_hex, encode(&tail_buf), "tail bytes mismatch");

            let mut full_buf = header_buf.clone();
            full_buf.extend_from_slice(&tail_buf);
            let decoded = T::decode(&mut &full_buf[..], 0).expect("decode failed");
            assert_eq!(decoded, *value, "decoded value mismatch");
        }

        fn assert_roundtrip<T>(value: &T)
        where
            T: Encoder<LittleEndian, 4, false, Ctx = EncodingContext> + PartialEq + std::fmt::Debug,
        {
            let mut ctx = EncodingContext::default();
            value.header_size(&mut ctx).expect("header_size failed");

            let mut header_buf = BytesMut::new();
            value
                .encode_header(&mut header_buf, &mut ctx)
                .expect("encode_header failed");

            let mut tail_buf = BytesMut::new();
            value
                .encode_tail(&mut tail_buf, &mut ctx)
                .expect("encode_tail failed");

            let mut full_buf = header_buf.clone();
            full_buf.extend_from_slice(&tail_buf);

            let decoded = T::decode(&mut &full_buf[..], 0).expect("decode failed");
            assert_eq!(decoded, *value, "decoded value mismatch");
        }

        #[allow(dead_code)]
        fn print_encoded(buf: &BytesMut) {
            println!("concat!(");
            for chunk in buf.chunks_exact(4) {
                let hex_chunk = encode(chunk);
                let decimal_value = read_u32_aligned::<LittleEndian, 4>(&chunk, 0).unwrap();
                println!("    \"{}\", // {}", hex_chunk, decimal_value);
            }
            println!(");");
        }

        mod map {
            use super::*;

            #[test]
            fn test_empty_hashmap() {
                let test_value: HashMap<u32, u32> = HashMap::new();
                let expected_header_hex = concat!(
                    "00000000", // length = 0
                    "14000000", // keys offset (20 bytes, immediately after header)
                    "00000000", // keys size = 0
                    "14000000", // values offset (20 bytes, immediately after keys)
                    "00000000"  // values size = 0
                );
                let expected_tail_hex = concat!(
                // No keys, no values
                );

                assert_codec(expected_header_hex, expected_tail_hex, &test_value);
            }

            #[test]
            fn test_simple_hashmap() {
                let test_value = HashMap::from([(100, 20), (3, 5), (1000, 60)]);
                let expected_header_hex = concat!(
                    "03000000", // length
                    "14000000", // keys offset (20 bytes)
                    "0c000000", // keys size (3 keys x 4 bytes)
                    "20000000", // values offset (32 bytes)
                    "0c000000"  // values size (3 values x 4 bytes)
                );
                let expected_tail_hex = concat!(
                    "03000000", "64000000", "e8030000", // Keys
                    "05000000", "14000000", "3c000000" // Values
                );

                assert_codec(expected_header_hex, expected_tail_hex, &test_value);
            }

            #[test]
            fn test_hashmap_nested() {
                let test_value = HashMap::from([
                    (100, HashMap::from([(1u32, 2u32), (3u32, 4u32)])),
                    (1000, HashMap::from([(7u32, 8u32), (9u32, 4u32)])),
                ]);

                let expected_header_hex = concat!(
                    "02000000", // 2    [0]  - outter length
                    "34000000", // 52   [1]  - keys offset          52 / 4 = 13 (0+13)   -> [13]
                    "08000000", // 8    [2]  - keys size (2*4)
                    "02000000", // 2    [3]  - m0 length
                    "30000000", // 48   [4]  - keys offset          48 / 4 = 12 (3 + 12) -> [15]
                    "08000000", // 8    [5]  - keys size (2*4B)
                    "38000000", // 56   [6]  - values offset.       56 / 4 = 14 (3 + 14) -> [17]
                    "08000000", // 8    [7]  - values size (2*4B)
                    "02000000", // 2    [8]  - m1 length
                    "2c000000", // 44   [9]  - keys offset.         44 / 4 = 11 (8 + 11) -> [19]
                    "08000000", // 8    [10] - keys size (2*4B)
                    "34000000", // 52   [11] - values offset.       52 / 4 = 13 (8 + 13) -> [21]
                    "08000000", // 8    [12] - values size (2*4B)
                );
                let expected_tail_hex = concat!(
                    "64000000", // [13] 100 -- ok
                    "e8030000", // [14] 1000 -- ok
                    "01000000", // [15] 1 -- ok m[0][k0]
                    "03000000", // [16] 3 -- ok m[0][k1]
                    "02000000", // [17] 2 -- ok m[0][v0]
                    "04000000", // [18] 4 -- ok m[0][v1]
                    "07000000", // [19] 7 -- ok m[1][k0]
                    "09000000", // [20] 9 -- ok m[1][k1]
                    "08000000", // [21] 8 -- ok m[1][v0]
                    "04000000", // [22] 4 -- ok m[1][v1]
                );

                assert_codec(expected_header_hex, expected_tail_hex, &test_value);
            }

            #[test]
            fn test_hashmap_roundtrip() {
                let with_empty = vec![
                    HashMap::from([(1, 2), (3, 4)]),
                    HashMap::new(),
                    HashMap::from([(7, 8), (9, 4)]),
                ];

                assert_roundtrip(&with_empty);

                let large_hashmap: HashMap<u32, u32> = (0..1000).map(|i| (i, i * 2)).collect();

                assert_roundtrip(&large_hashmap)
            }
        }

        mod hash_set {
            use super::*;
            use hashbrown::HashSet;
            //
            #[test]
            fn test_empty_hashset() {
                let value: HashSet<u32> = HashSet::new();
                assert_codec(
                    "000000000c00000000000000", // len = 0, offset = 12, size = 0
                    "",                         // no tail
                    &value,
                );
            }

            #[test]
            fn test_hashset_u32_codec() {
                let value = HashSet::from([3u32, 1, 4, 5, 2]);
                assert_codec(
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
            fn test_hashset_sorting() {
                let value1: HashSet<u32> = HashSet::from([3u32, 1, 2]);
                let value2: HashSet<u32> = HashSet::from([2u32, 3, 1]);

                let mut ctx1 = EncodingContext::default();
                <HashSet<u32> as Encoder<LittleEndian, 4, false>>::header_size(&value1, &mut ctx1)
                    .unwrap();

                let mut header_buf1 = BytesMut::new();
                <HashSet<u32> as Encoder<LittleEndian, 4, false>>::encode_header(
                    &value1,
                    &mut header_buf1,
                    &mut ctx1,
                )
                .unwrap();

                let mut ctx2 = EncodingContext::default();
                <HashSet<u32> as Encoder<LittleEndian, 4, false>>::header_size(&value2, &mut ctx2)
                    .unwrap();

                let mut header_buf2 = BytesMut::new();
                <HashSet<u32> as Encoder<LittleEndian, 4, false>>::encode_header(
                    &value2,
                    &mut header_buf2,
                    &mut ctx2,
                )
                .unwrap();

                assert_eq!(
                    header_buf1, header_buf2,
                    "Headers mismatch, sorting inconsistency"
                );

                let mut tail_buf1 = BytesMut::new();
                <HashSet<u32> as Encoder<LittleEndian, 4, false>>::encode_tail(
                    &value1,
                    &mut tail_buf1,
                    &mut ctx1,
                )
                .unwrap();

                let mut tail_buf2 = BytesMut::new();
                <HashSet<u32> as Encoder<LittleEndian, 4, false>>::encode_tail(
                    &value2,
                    &mut tail_buf2,
                    &mut ctx2,
                )
                .unwrap();

                assert_eq!(
                    tail_buf1, tail_buf2,
                    "Tails mismatch, sorting inconsistency"
                );
            }
        }
    }
}
