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
    K: Encoder<B, ALIGN, false> + Ord + Eq + Hash + Debug,
    V: Encoder<B, ALIGN, false> + Debug,
{
    const HEADER_SIZE: usize = align_up::<ALIGN>(size_of::<u32>()) * 5; // length (4) + keys_header (8) + values_header (8)
    const IS_DYNAMIC: bool = true; // length

    fn header_size(&self) -> usize {
        let mut total_size = Self::HEADER_SIZE;

        if V::IS_DYNAMIC {
            // Exclude redundant offset and size fields for dynamic values
            total_size -= 2 * align_up::<ALIGN>(4);

            // Add headers of nested dynamic values
            total_size += self
                .values()
                .map(|value| value.header_size())
                .sum::<usize>();
        }

        total_size
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

        // TODO: we don't need to sort inside header (add test for nested compact abi)
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
            for (_, value) in &entries {
                value.encode_header(out, ctx)?;
            }
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

/// HashMap implementation for Solidity ABI
///
/// Encoding structure:
/// - header: offset (32 bytes)
/// - tail:
///   - length (32 bytes)
///   - keys_offset (32 bytes, relative to tail start)
///   - values_offset (32 bytes, relative to tail start)
///   - keys data (encoded as Vec<K>)
///   - values data (encoded as Vec<V>)
impl<K, V, B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, true> for HashMap<K, V>
where
    K: Encoder<B, ALIGN, true> + Clone + Eq + Hash + Ord + core::fmt::Debug,
    V: Encoder<B, ALIGN, true> + Clone + core::fmt::Debug,
{
    const HEADER_SIZE: usize = align_up::<ALIGN>(32); // Only offset in header
    const IS_DYNAMIC: bool = true;

    /// Adds the HashMap header size to the encoding context
    fn header_size(&self) -> usize {
        Self::HEADER_SIZE
    }

    /// Encodes the header - writes only the offset to the HashMap data
    /// Encodes the header - writes only the offset to the HashMap data
    fn encode_header(
        &self,
        buf: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        // Calculate offset to HashMap data
        let offset = (ctx.hdr_size - ctx.hdr_ptr) + ctx.data_ptr;
        write_u32_aligned::<B, 32>(buf, offset);
        ctx.hdr_ptr += 32;

        // Reserve space for HashMap data in tail
        let mut temp_ctx = EncodingContext::new();
        let data_size = self.tail_size(&mut temp_ctx)?;
        ctx.data_ptr += data_size as u32;

        Ok(32)
    }

    /// Encodes the tail - writes the actual HashMap data
    fn encode_tail(
        &self,
        buf: &mut impl BufMut,
        _ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        let tail_start = buf.remaining_mut();

        // 1. Write HashMap length
        write_u32_aligned::<B, 32>(buf, self.len() as u32);

        if self.is_empty() {
            // Empty HashMap only needs length field
            return Ok(32);
        }

        // 2. Sort entries by key for deterministic encoding
        let mut entries: Vec<_> = self.iter().collect();
        entries.sort_by_key(|(k, _)| *k);

        // 3. Prepare keys and values vectors
        let keys: Vec<K> = entries.iter().map(|(k, _)| (*k).clone()).collect();
        let values: Vec<V> = entries.iter().map(|(_, v)| (*v).clone()).collect();

        // 4. Calculate sizes for proper offset values
        // Keys start after: length (32) + keys_offset (32) + values_offset (32) = 96 bytes
        let keys_start_offset = 96u32;

        // Calculate size of keys vector (including its length field and data)
        let mut temp_ctx = EncodingContext::new();
        let keys_total_size = keys.tail_size(&mut temp_ctx)?;

        // Values start after keys data
        let values_start_offset = keys_start_offset + keys_total_size as u32;

        // 5. Write offsets (relative to the start of HashMap tail)
        write_u32_aligned::<B, 32>(buf, keys_start_offset);
        write_u32_aligned::<B, 32>(buf, values_start_offset);

        // 6. Write keys and values as vectors
        // IMPORTANT: Vec<K> and Vec<V> are treated as dynamic elements of HashMap
        // Therefore, we need a fresh context for them, similar to how Vec handles dynamic elements
        let mut local_ctx = EncodingContext::new();

        // Write keys vector (it will write its own length and elements)
        keys.encode_tail(buf, &mut local_ctx)?;

        // Write values vector (it will write its own length and elements)
        values.encode_tail(buf, &mut local_ctx)?;

        Ok(tail_start - buf.remaining_mut())
    }

    /// Decodes a HashMap from Solidity ABI encoded buffer
    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        println!("=== HashMap::decode START at offset {} ===", offset);

        // Read offset to HashMap data
        let data_offset = read_u32_aligned::<B, 32>(buf, offset)? as usize;
        println!("Read data offset: {} from position {}", data_offset, offset);

        // Read HashMap length
        let length = read_u32_aligned::<B, 32>(buf, data_offset)? as usize;
        println!(
            "Read HashMap length: {} from position {}",
            length, data_offset
        );

        if length == 0 {
            return Ok(HashMap::new());
        }

        // Create chunk starting from HashMap data (like Vec does)
        let chunk = &buf.chunk()[data_offset..];
        println!("Values chunk: {:?}", hex::encode(chunk));
        // Decode vectors using relative offsets
        println!("\n--- Decoding keys vector ---");
        let keys: Vec<K> = Vec::decode(&chunk, 32)?;
        println!("Decoded {} keys", keys.len());

        println!("\n--- Decoding values vector ---");
        let values: Vec<V> = Vec::decode(&chunk, 64)?;
        println!("Decoded {} values", values.len());

        // Build HashMap
        let mut result = HashMap::with_capacity(length);
        for (key, value) in keys.into_iter().zip(values.into_iter()) {
            result.insert(key, value);
        }

        println!("=== HashMap::decode END ===");
        Ok(result)
    }

    /// Calculates the size of tail data using ByteCounter
    fn tail_size(&self, _ctx: &mut EncodingContext) -> Result<usize, CodecError> {
        if self.is_empty() {
            return Ok(32);
        }

        // Base size: length + 2 offsets + 2 vec lengths
        let mut size = 96 + 64; // 160 bytes

        // Add size for all elements
        let element_count = self.len();

        // Keys size
        if K::IS_DYNAMIC {
            size += element_count * 32; // Offsets
                                        // Need to iterate for dynamic sizes
            for (key, _) in self.iter() {
                let mut temp_ctx = EncodingContext::new();
                size += key.tail_size(&mut temp_ctx)?;
            }
        } else {
            // For static types: just count * 32
            size += element_count * 32;
        }

        // Values size
        if V::IS_DYNAMIC {
            size += element_count * 32; // Offsets
                                        // Need to iterate for dynamic sizes
            for (_, value) in self.iter() {
                let mut temp_ctx = EncodingContext::new();
                size += value.tail_size(&mut temp_ctx)?;
            }
        } else {
            // For static types: just count * 32
            size += element_count * 32;
        }

        Ok(size)
    }

    /// Returns the number of bytes this HashMap would take when encoded
    fn len(&self) -> usize {
        // This is used for static size calculations
        // For dynamic types like HashMap, this is just the header size
        Self::HEADER_SIZE
    }
}

impl<T, B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, false> for HashSet<T>
where
    T: Encoder<B, ALIGN, false> + Ord + Eq + Hash + Debug,
{
    const HEADER_SIZE: usize = size_of::<u32>() * 3; // len (4) + keys_offset (4) + keys_size (4)
    const IS_DYNAMIC: bool = true;

    fn header_size(&self) -> usize {
        let mut total_size = Self::HEADER_SIZE;

        if T::IS_DYNAMIC {
            // Dynamic keys store offset/size in nested headers; exclude them here
            total_size -= 2 * align_up::<ALIGN>(4);
            total_size += self.iter().map(|key| key.header_size()).sum::<usize>();
        }

        total_size
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
            for key in elements {
                key.encode_header(out, ctx)?;
            }
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

/// HashSet implementation for Solidity ABI
///
/// Encoding structure (identical to Vec):
/// - header: offset (32 bytes)
/// - tail:
///   - length (32 bytes)
///   - for static elements: elements directly
///   - for dynamic elements: offsets followed by element data
impl<T, B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, true> for HashSet<T>
where
    T: Encoder<B, ALIGN, true> + Eq + Hash + Ord + Debug,
{
    const HEADER_SIZE: usize = 32;
    const IS_DYNAMIC: bool = true;

    /// Adds the HashSet header size to the encoding context
    fn header_size(&self) -> usize {
        Self::HEADER_SIZE
    }

    /// Encodes the header - writes only the offset to the HashSet data
    fn encode_header(
        &self,
        buf: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        // Calculate offset to HashSet data
        let offset = (ctx.hdr_size - ctx.hdr_ptr) + ctx.data_ptr;
        write_u32_aligned::<B, 32>(buf, offset);
        ctx.hdr_ptr += 32;

        // Reserve space for HashSet data in tail
        let data_size = self.tail_size(ctx)?;
        ctx.data_ptr += data_size as u32;

        Ok(32)
    }

    /// Encodes the tail - writes the actual HashSet data
    fn encode_tail(
        &self,
        buf: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        let tail_start = buf.remaining_mut();

        // Write length
        write_u32_aligned::<B, 32>(buf, self.len() as u32);

        if self.is_empty() {
            return Ok(32);
        }

        // Sort elements for deterministic encoding
        // Only collect references to avoid cloning
        let mut sorted_refs: Vec<&T> = self.iter().collect();
        sorted_refs.sort();

        if !T::IS_DYNAMIC {
            // Static elements - write them directly
            for element in sorted_refs {
                element.encode_header(buf, ctx)?;
            }
        } else {
            // Dynamic elements - two-phase encoding like Vec

            // Save context and create local one
            let old_hdr_ptr = ctx.hdr_ptr;
            let old_hdr_size = ctx.hdr_size;
            let old_data_ptr = ctx.data_ptr;

            // Local context for offset calculations
            ctx.hdr_ptr = 0;
            ctx.hdr_size = (self.len() * 32) as u32;
            ctx.data_ptr = 0;

            // Phase 1: Write offsets
            for element in &sorted_refs {
                let element_offset = ctx.hdr_size + ctx.data_ptr;
                write_u32_aligned::<B, 32>(buf, element_offset);

                // Calculate element size without allocating temp context repeatedly
                let element_size = if T::IS_DYNAMIC {
                    let mut temp_ctx = EncodingContext::new();
                    element.tail_size(&mut temp_ctx)?
                } else {
                    32
                };

                ctx.data_ptr += element_size as u32;
            }

            // Phase 2: Write element data
            for element in sorted_refs {
                element.encode_tail(buf, ctx)?;
            }

            // Restore context
            ctx.hdr_ptr = old_hdr_ptr;
            ctx.hdr_size = old_hdr_size;
            ctx.data_ptr = old_data_ptr + (tail_start - buf.remaining_mut()) as u32;
        }

        Ok(tail_start - buf.remaining_mut())
    }

    /// Decodes a HashSet from Solidity ABI encoded buffer
    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        // Reuse Vec decoder since the encoding format is identical
        let vec = Vec::<T>::decode(buf, offset)?;

        // Convert to HashSet, which automatically handles uniqueness
        let set: HashSet<T> = vec.into_iter().collect();

        // Note: If the encoded data had duplicates, they will be removed
        // This is not an error in HashSet context

        Ok(set)
    }

    /// Calculates the size of tail data without allocations
    fn tail_size(&self, _ctx: &mut EncodingContext) -> Result<usize, CodecError> {
        if self.is_empty() {
            return Ok(32);
        }

        // Base size: length field
        let mut size = 32;

        if T::IS_DYNAMIC {
            // Add space for offsets
            size += self.len() * 32;

            // Add space for each element's data
            for element in self.iter() {
                let mut temp_ctx = EncodingContext::new();
                size += element.tail_size(&mut temp_ctx)?;
            }
        } else {
            // Static elements: fixed size per element
            size += self.len() * 32;
        }

        Ok(size)
    }

    /// Returns the header size for this HashSet
    fn len(&self) -> usize {
        Self::HEADER_SIZE
    }
}

#[cfg(test)]
mod tests {
    mod compact {
        use crate::optimized::{ctx::EncodingContext, encoder::Encoder, utils::test_utils::*};
        use byteorder::LittleEndian;
        use bytes::BytesMut;
        use core::hash::Hash;
        use hashbrown::HashMap;

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

                assert_codec_compact(expected_header_hex, expected_tail_hex, &test_value);
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

                assert_codec_compact(expected_header_hex, expected_tail_hex, &test_value);
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

                assert_codec_compact(expected_header_hex, expected_tail_hex, &test_value);
            }

            #[test]
            fn test_hashmap_roundtrip() {
                let with_empty = vec![
                    HashMap::from([(1, 2), (3, 4)]),
                    HashMap::new(),
                    HashMap::from([(7, 8), (9, 4)]),
                ];

                assert_roundtrip_compact(&with_empty);

                let large_hashmap: HashMap<u32, u32> = (0..1000).map(|i| (i, i * 2)).collect();

                assert_roundtrip_compact(&large_hashmap)
            }
        }

        mod hash_set {
            use super::*;
            use hashbrown::HashSet;
            //
            #[test]
            fn test_empty_hashset() {
                let value: HashSet<u32> = HashSet::new();
                assert_codec_compact(
                    "000000000c00000000000000", // len = 0, offset = 12, size = 0
                    "",                         // no tail
                    &value,
                );
            }

            #[test]
            fn test_hashset_u32_codec() {
                let value = HashSet::from([3u32, 1, 4, 5, 2]);
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
            fn test_hashset_sorting() {
                let value1: HashSet<u32> = HashSet::from([3u32, 1, 2]);
                let value2: HashSet<u32> = HashSet::from([2u32, 3, 1]);

                let mut ctx1 = EncodingContext::default();
                ctx1.hdr_size =
                    <HashSet<u32> as Encoder<LittleEndian, 4, false>>::header_size(&value2) as u32;

                let mut header_buf1 = BytesMut::new();
                <HashSet<u32> as Encoder<LittleEndian, 4, false>>::encode_header(
                    &value1,
                    &mut header_buf1,
                    &mut ctx1,
                )
                .unwrap();

                let mut ctx2 = EncodingContext::default();
                ctx2.hdr_size =
                    <HashSet<u32> as Encoder<LittleEndian, 4, false>>::header_size(&value2) as u32;
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

    mod sol {
        use crate::optimized::utils::test_utils::{assert_codec_sol, print_encoded};
        use byteorder::BigEndian;
        use hashbrown::HashMap;

        mod map {
            use super::*;
            #[test]
            fn simple_map() {
                let v = HashMap::from([(1, 5), (10u32, 20u32), (100, 60)]);
                assert_codec_sol(
                    concat!(
                        "0000000000000000000000000000000000000000000000000000000000000020", /* [0x0000] 0 = 32 */
                    ),
                    concat!(
                        "0000000000000000000000000000000000000000000000000000000000000003", /* [0x0000] 0 = 3 */
                        "0000000000000000000000000000000000000000000000000000000000000060", /* [0x0020] 32 = 96 */
                        "00000000000000000000000000000000000000000000000000000000000000e0", /* [0x0040] 64 = 224 */
                        "0000000000000000000000000000000000000000000000000000000000000003", /* [0x0060] 96 = 3 */
                        "0000000000000000000000000000000000000000000000000000000000000001", /* [0x0080] 128 = 1 */
                        "000000000000000000000000000000000000000000000000000000000000000a", /* [0x00a0] 160 = 10 */
                        "0000000000000000000000000000000000000000000000000000000000000064", /* [0x00c0] 192 = 100 */
                        "0000000000000000000000000000000000000000000000000000000000000003", /* [0x00e0] 224 = 3 */
                        "0000000000000000000000000000000000000000000000000000000000000005", /* [0x0100] 256 = 5 */
                        "0000000000000000000000000000000000000000000000000000000000000014", /* [0x0120] 288 = 20 */
                        "000000000000000000000000000000000000000000000000000000000000003c", /* [0x0140] 320 = 60 */
                    ),
                    &v,
                );
            }

            #[test]
            fn nested_map() {
                let test_value = HashMap::from([
                    (100, HashMap::from([(1u32, 2u32), (3u32, 4u32)])),
                    (1000, HashMap::from([(7u32, 8u32), (9u32, 4u32)])),
                ]);
                print_encoded::<BigEndian, 32>(hex::decode("0000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000c00000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000006400000000000000000000000000000000000000000000000000000000000003e80000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000001600000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000c00000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000030000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000c0000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000070000000000000000000000000000000000000000000000000000000000000009000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000004").unwrap());

                assert_codec_sol(
                    concat!(
                        "0000000000000000000000000000000000000000000000000000000000000020", /* offset = 32 */
                    ),
                    concat!(
                        "0000000000000000000000000000000000000000000000000000000000000002", /* [0x0000] 0 = 2 */
                        "0000000000000000000000000000000000000000000000000000000000000060", /* [0x0020] 32 = 96 */
                        "00000000000000000000000000000000000000000000000000000000000000c0", /* [0x0040] 64 = 192 */
                        "0000000000000000000000000000000000000000000000000000000000000002", /* [0x0060] 96 = 2 */
                        "0000000000000000000000000000000000000000000000000000000000000064", /* [0x0080] 128 = 100 */
                        "00000000000000000000000000000000000000000000000000000000000003e8", /* [0x00a0] 160 = 1000 */
                        "0000000000000000000000000000000000000000000000000000000000000002", /* [0x00c0] 192 = 2 */
                        "0000000000000000000000000000000000000000000000000000000000000040", /* [0x00e0] 224 = 64 */
                        "0000000000000000000000000000000000000000000000000000000000000160", /* [0x0100] 256 = 352 */
                        "0000000000000000000000000000000000000000000000000000000000000002", /* [0x0120] 288 = 2 */
                        "0000000000000000000000000000000000000000000000000000000000000060", /* [0x0140] 320 = 96 */
                        "00000000000000000000000000000000000000000000000000000000000000c0", /* [0x0160] 352 = 192 */
                        "0000000000000000000000000000000000000000000000000000000000000002", /* [0x0180] 384 = 2 */
                        "0000000000000000000000000000000000000000000000000000000000000001", /* [0x01a0] 416 = 1 */
                        "0000000000000000000000000000000000000000000000000000000000000003", /* [0x01c0] 448 = 3 */
                        "0000000000000000000000000000000000000000000000000000000000000002", /* [0x01e0] 480 = 2 */
                        "0000000000000000000000000000000000000000000000000000000000000002", /* [0x0200] 512 = 2 */
                        "0000000000000000000000000000000000000000000000000000000000000004", /* [0x0220] 544 = 4 */
                        "0000000000000000000000000000000000000000000000000000000000000002", /* [0x0240] 576 = 2 */
                        "0000000000000000000000000000000000000000000000000000000000000060", /* [0x0260] 608 = 96 */
                        "00000000000000000000000000000000000000000000000000000000000000c0", /* [0x0280] 640 = 192 */
                        "0000000000000000000000000000000000000000000000000000000000000002", /* [0x02a0] 672 = 2 */
                        "0000000000000000000000000000000000000000000000000000000000000007", /* [0x02c0] 704 = 7 */
                        "0000000000000000000000000000000000000000000000000000000000000009", /* [0x02e0] 736 = 9 */
                        "0000000000000000000000000000000000000000000000000000000000000002", /* [0x0300] 768 = 2 */
                        "0000000000000000000000000000000000000000000000000000000000000008", /* [0x0320] 800 = 8 */
                        "0000000000000000000000000000000000000000000000000000000000000004", /* [0x0340] 832 = 4 */
                    ),
                    &test_value,
                );
            }
        }

        mod hash_set {
            use super::*;
            use crate::optimized::{
                encoder::Encoder,
                utils::test_utils::assert_roundtrip_sol,
            };
            use hashbrown::HashSet;

            #[test]
            fn test_empty_hashset() {
                let value: HashSet<u32> = HashSet::new();
                assert_codec_sol(
                    "0000000000000000000000000000000000000000000000000000000000000020", /* offset = 32 */
                    "0000000000000000000000000000000000000000000000000000000000000000", /* length = 0 */
                    &value,
                );
            }

            #[test]
            fn test_hashset_u32() {
                let value = HashSet::from([3u32, 1, 4, 5, 2]);
                // HashSet will be sorted as [1, 2, 3, 4, 5]
                assert_codec_sol(
                    concat!(
                        "0000000000000000000000000000000000000000000000000000000000000020", /* offset = 32 */
                    ),
                    concat!(
                        "0000000000000000000000000000000000000000000000000000000000000005", /* length = 5 */
                        "0000000000000000000000000000000000000000000000000000000000000001", // 1
                        "0000000000000000000000000000000000000000000000000000000000000002", // 2
                        "0000000000000000000000000000000000000000000000000000000000000003", // 3
                        "0000000000000000000000000000000000000000000000000000000000000004", // 4
                        "0000000000000000000000000000000000000000000000000000000000000005", // 5
                    ),
                    &value,
                );
            }

            #[test]
            fn test_hashset_sorting_deterministic() {
                // Different insertion orders should produce same encoding
                let value1: HashSet<u32> = HashSet::from([3u32, 1, 2]);
                let value2: HashSet<u32> = HashSet::from([2u32, 3, 1]);
                let value3: HashSet<u32> = HashSet::from([1u32, 3, 2]);

                // All should encode to sorted order [1, 2, 3]
                let expected_tail = concat!(
                    "0000000000000000000000000000000000000000000000000000000000000003", /* length = 3 */
                    "0000000000000000000000000000000000000000000000000000000000000001", // 1
                    "0000000000000000000000000000000000000000000000000000000000000002", // 2
                    "0000000000000000000000000000000000000000000000000000000000000003", // 3
                );

                // Test all three produce same encoding
                for value in [value1, value2, value3] {
                    assert_codec_sol(
                        "0000000000000000000000000000000000000000000000000000000000000020",
                        expected_tail,
                        &value,
                    );
                }
            }

            #[test]
            fn test_hashset_uniqueness() {
                // Test that duplicate values are handled correctly
                let vec_with_dups = vec![1u32, 2, 3, 2, 1, 3];
                let set_from_vec: HashSet<u32> = vec_with_dups.into_iter().collect();

                assert_eq!(set_from_vec.len(), 3); // Only unique values

                assert_codec_sol(
                    "0000000000000000000000000000000000000000000000000000000000000020",
                    concat!(
                        "0000000000000000000000000000000000000000000000000000000000000003", /* length = 3 */
                        "0000000000000000000000000000000000000000000000000000000000000001", // 1
                        "0000000000000000000000000000000000000000000000000000000000000002", // 2
                        "0000000000000000000000000000000000000000000000000000000000000003", // 3
                    ),
                    &set_from_vec,
                );
            }
            #[test]
            fn test_hashset_roundtrip() {
                // Test various types roundtrip correctly
                let sets = vec![
                    HashSet::from([1u32, 2, 3, 4, 5]),
                    HashSet::from([100, 200, 300]),
                    HashSet::from([u32::MAX, u32::MIN, 42]),
                ];

                for original in sets {
                    // Full roundtrip test using assert_roundtrip_sol
                    assert_roundtrip_sol(&original);
                }
            }
        }
    }
}
