use crate::{
    alloc::string::ToString,
    bytes_codec::{read_bytes, read_bytes_header},
    encoder::{align_up, read_u32_aligned, write_u32_aligned, Encoder},
    error::{CodecError, DecodingError},
};
use alloc::{fmt::Debug, vec::Vec};
use byteorder::ByteOrder;
use bytes::{Buf, BufMut, BytesMut};

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
    const HEADER_SIZE: usize = size_of::<u32>() * 3;
    const IS_DYNAMIC: bool = true;

    fn encode(&self, buf: &mut impl BufMut, offset: usize) -> Result<usize, CodecError> {
        let aligned_elem_size = align_up::<ALIGN>(4);
        let aligned_header_size = aligned_elem_size * 3;

        // Calculate data size
        let elem_size = align_up::<ALIGN>(T::HEADER_SIZE);
        let data_size = elem_size * self.len();

        // Write length of the vector
        write_u32_aligned::<B, ALIGN>(buf, self.len() as u32);

        // Write offset (relative to current position)
        write_u32_aligned::<B, ALIGN>(buf, aligned_header_size as u32);

        // Write size
        write_u32_aligned::<B, ALIGN>(buf, data_size as u32);

        // Encode values
        let mut current_offset = offset + aligned_header_size;
        for obj in self.iter() {
            let encoded_size = obj.encode(buf, current_offset)?;
            current_offset += encoded_size;
        }

        Ok(aligned_header_size + data_size)
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let aligned_header_el_size = align_up::<ALIGN>(4);

        if buf.remaining() < offset + aligned_header_el_size {
            return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                expected: offset + aligned_header_el_size,
                found: buf.remaining(),
                msg: "failed to decode vector length".to_string(),
            }));
        }

        let data_len = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;
        if data_len == 0 {
            return Ok(Vec::new());
        }

        let mut result = Vec::with_capacity(data_len);
        let data = read_bytes::<B, ALIGN, false>(buf, offset + aligned_header_el_size)?;

        for i in 0..data_len {
            let elem_offset = i * align_up::<ALIGN>(T::HEADER_SIZE);
            let value = T::decode(&data, elem_offset)?;
            result.push(value);
        }

        Ok(result)
    }

    fn partial_decode(buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        read_bytes_header::<B, ALIGN, false>(buf, offset)
    }
}

use core::mem::MaybeUninit;

/// Solidity ABI encoder implementation for Vec<T>
///
/// # Encoding Rules:
/// - Static types (Vec<u32>): Single pass encoding
/// - Dynamic types (Vec<Vec<T>>): Two-pass encoding
///
/// # Memory Layout:
/// ```text
/// [offset: 32 bytes] -> points to array data (only for top-level)
/// [length: 32 bytes] -> number of elements
/// [offsets: n * 32 bytes] -> only for dynamic elements
/// [data: variable] -> actual element data
/// ```
impl<T, B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, true, false> for Vec<T>
where
    T: Default + Sized + Encoder<B, ALIGN, true, false> + Debug,
{
    const HEADER_SIZE: usize = 32;
    const IS_DYNAMIC: bool = true;

    fn encode(&self, buf: &mut impl BufMut, offset: usize) -> Result<usize, CodecError> {
        // offset == 0 means this is a top-level dynamic type
        // offset != 0 means this is nested inside another structure
        let is_top_level = offset == 0;

        if T::IS_DYNAMIC {
            encode_dynamic_two_pass::<T, B, ALIGN>(self, buf, is_top_level)
        } else {
            encode_static_single_pass::<T, B, ALIGN>(self, buf, is_top_level)
        }
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        // Read offset to array data (32 bytes aligned)
        let data_offset = if offset == 0 {
            // Top-level dynamic type: read offset from position 0
            read_u32_aligned::<B, ALIGN>(buf, 0)?
        } else {
            // Nested dynamic type: offset points directly to data
            offset as u32
        } as usize;

        // Read array length
        let length = read_u32_aligned::<B, ALIGN>(buf, data_offset)? as usize;

        if length == 0 {
            return Ok(Vec::new());
        }

        let mut result = Vec::with_capacity(length);

        if T::IS_DYNAMIC {
            // Dynamic elements: read via offset pointers
            let offsets_start = data_offset + 32; // After length field

            for i in 0..length {
                let offset_position = offsets_start + i * 32;
                let element_offset = read_u32_aligned::<B, ALIGN>(buf, offset_position)? as usize;
                // Element offset is relative to the start of offset zone
                let absolute_offset = offsets_start + element_offset;
                result.push(T::decode(buf, absolute_offset)?);
            }
        } else {
            // Static elements: read sequentially
            let mut current_offset = data_offset + 32; // After length field
            let element_size = align_up::<ALIGN>(T::HEADER_SIZE);

            for _ in 0..length {
                result.push(T::decode(buf, current_offset)?);
                current_offset += element_size;
            }
        }

        Ok(result)
    }

    fn partial_decode(buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        let data_offset = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;
        let length = read_u32_aligned::<B, ALIGN>(buf, data_offset)? as usize;

        // Conservative size estimate
        let element_size = if T::IS_DYNAMIC {
            32
        } else {
            align_up::<ALIGN>(T::HEADER_SIZE)
        };
        let total_size = 32 + 32 + length * element_size;

        Ok((offset, total_size))
    }
}

// Stack-allocated buffer for small arrays to avoid heap allocation
const STACK_BUFFER_SIZE: usize = 16;

/// Size information for two-pass encoding
#[derive(Debug, Clone, Copy)]
struct SizeInfo {
    /// Total encoded size of the element
    encoded_size: usize,
}

/// Single-pass encoding for static element types
///
/// # Example: Vec<u32> = [1, 2, 3]
/// ```text
/// Top-level:
/// Position  | Value | Description
/// ----------|-------|-------------
/// 0x00      | 32    | offset to data (only for top-level)
/// 0x20      | 3     | array length
/// 0x40      | 1     | element[0]
/// 0x60      | 2     | element[1]
/// 0x80      | 3     | element[2]
///
/// Nested:
/// 0x00      | 3     | array length (no offset!)
/// 0x20      | 1     | element[0]
/// 0x40      | 2     | element[1]
/// 0x60      | 3     | element[2]
/// ```
#[inline]
fn encode_static_single_pass<T, B: ByteOrder, const ALIGN: usize>(
    vec: &[T],
    buf: &mut impl BufMut,
    is_top_level: bool,
) -> Result<usize, CodecError>
where
    T: Encoder<B, ALIGN, true, false>,
{
    let mut total_size = 0;

    // Write offset only for top-level arrays
    if is_top_level {
        write_u32_aligned::<B, ALIGN>(buf, 32);
        total_size += 32;
    }

    // Write length
    write_u32_aligned::<B, ALIGN>(buf, vec.len() as u32);
    total_size += 32;

    // Write elements sequentially
    for element in vec.iter() {
        // Pass offset=1 to indicate nested context
        total_size += element.encode(buf, 1)?;
    }

    Ok(total_size)
}

/// Two-pass encoding for dynamic element types
///
/// # Example: Vec<Vec<u32>> = [[1, 2, 3], [4, 5]]
/// ```text
/// PASS 1: Calculate sizes
/// - vec[0] size = 32 (length) + 3*32 (elements) = 128 (no offset for nested!)
/// - vec[1] size = 32 (length) + 2*32 (elements) = 96 (no offset for nested!)
///
/// PASS 2: Write with calculated offsets
/// Position  | Value | Description
/// ----------|-------|-------------
/// 0x00      | 32    | offset to array data (only for top-level)
/// 0x20      | 2     | array length
/// 0x40      | 64    | offset[0] (relative to 0x40)
/// 0x60      | 192   | offset[1] (relative to 0x40)
/// 0x80      | ...   | data for vec[0]
/// 0x100     | ...   | data for vec[1]
/// ```
fn encode_dynamic_two_pass<T, B: ByteOrder, const ALIGN: usize>(
    vec: &[T],
    buf: &mut impl BufMut,
    is_top_level: bool,
) -> Result<usize, CodecError>
where
    T: Encoder<B, ALIGN, true, false>,
{
    // Optimization: use stack buffer for small arrays
    if vec.len() <= STACK_BUFFER_SIZE {
        encode_dynamic_stack_optimized::<T, B, ALIGN>(vec, buf, is_top_level)
    } else {
        encode_dynamic_heap::<T, B, ALIGN>(vec, buf, is_top_level)
    }
}

/// Stack-optimized version for small arrays (no heap allocation)
#[inline]
fn encode_dynamic_stack_optimized<T, B: ByteOrder, const ALIGN: usize>(
    vec: &[T],
    buf: &mut impl BufMut,
    is_top_level: bool,
) -> Result<usize, CodecError>
where
    T: Encoder<B, ALIGN, true, false>,
{
    let mut sizes = [SizeInfo { encoded_size: 0 }; STACK_BUFFER_SIZE];

    // PASS 1: Calculate sizes
    for (i, element) in vec.iter().enumerate() {
        sizes[i].encoded_size = calculate_encoded_size::<T, B, ALIGN>(element)?;
    }

    // Write using calculated sizes
    write_with_sizes::<T, B, ALIGN>(vec, buf, &sizes[..vec.len()], is_top_level)
}

/// Heap version for large arrays
#[inline]
fn encode_dynamic_heap<T, B: ByteOrder, const ALIGN: usize>(
    vec: &[T],
    buf: &mut impl BufMut,
    is_top_level: bool,
) -> Result<usize, CodecError>
where
    T: Encoder<B, ALIGN, true, false>,
{
    // PASS 1: Calculate sizes of all elements
    let sizes: Vec<SizeInfo> = vec
        .iter()
        .map(|element| {
            Ok(SizeInfo {
                encoded_size: calculate_encoded_size::<T, B, ALIGN>(element)?,
            })
        })
        .collect::<Result<Vec<_>, CodecError>>()?;

    // Write using calculated sizes
    write_with_sizes::<T, B, ALIGN>(vec, buf, &sizes, is_top_level)
}

/// Common write logic using pre-calculated sizes
#[inline(always)]
fn write_with_sizes<T, B: ByteOrder, const ALIGN: usize>(
    vec: &[T],
    buf: &mut impl BufMut,
    sizes: &[SizeInfo],
    is_top_level: bool,
) -> Result<usize, CodecError>
where
    T: Encoder<B, ALIGN, true, false>,
{
    let mut total_size = 0;

    // Write header offset only for top-level arrays
    if is_top_level {
        write_u32_aligned::<B, ALIGN>(buf, 32);
        total_size += 32;
    }

    // Write array length
    write_u32_aligned::<B, ALIGN>(buf, vec.len() as u32);
    total_size += 32;

    // Calculate and write element offsets
    let offsets_size = vec.len() * 32;
    total_size += offsets_size;

    // Element offsets are relative to the start of the offset zone
    let mut element_offset = offsets_size; // First element starts after all offsets

    for size_info in sizes.iter() {
        write_u32_aligned::<B, ALIGN>(buf, element_offset as u32);
        element_offset += size_info.encoded_size;
    }

    // PASS 2: Write actual element data
    // For nested vectors, pass offset=1 to indicate they are nested
    for element in vec.iter() {
        element.encode(buf, 1)?; // offset=1 means nested
    }

    // Add sizes of all elements
    total_size += sizes.iter().map(|s| s.encoded_size).sum::<usize>();

    Ok(total_size)
}

/// Calculate encoded size without actually encoding
/// Uses a counting buffer to avoid memory allocation
#[inline]
fn calculate_encoded_size<T, B: ByteOrder, const ALIGN: usize>(
    element: &T,
) -> Result<usize, CodecError>
where
    T: Encoder<B, ALIGN, true, false>,
{
    let mut counter = ByteCounter::new();
    // Pass offset=1 to calculate size for nested context
    element.encode(&mut counter, 1)?;
    Ok(counter.count())
}

/// A BufMut implementation that only counts bytes without storing them
struct ByteCounter {
    count: usize,
}

impl ByteCounter {
    #[inline(always)]
    fn new() -> Self {
        Self { count: 0 }
    }

    #[inline(always)]
    fn count(&self) -> usize {
        self.count
    }
}

unsafe impl BufMut for ByteCounter {
    #[inline(always)]
    fn remaining_mut(&self) -> usize {
        usize::MAX
    }

    #[inline(always)]
    unsafe fn advance_mut(&mut self, cnt: usize) {
        self.count += cnt;
    }

    #[inline(always)]
    fn chunk_mut(&mut self) -> &mut bytes::buf::UninitSlice {
        // Return a dummy buffer - we only count, never write
        static mut DUMMY: [MaybeUninit<u8>; 1024] = [MaybeUninit::uninit(); 1024];
        unsafe {
            // Convert [MaybeUninit<u8>] to UninitSlice
            bytes::buf::UninitSlice::from_raw_parts_mut(DUMMY.as_mut_ptr() as *mut u8, 1024)
        }
    }

    // Optimize common write operations to just count bytes
    #[inline(always)]
    fn put_u32(&mut self, _: u32) {
        self.count += 4;
    }

    #[inline(always)]
    fn put_slice(&mut self, src: &[u8]) {
        self.count += src.len();
    }
}

// ============================================================================
// DETAILED EXAMPLE: How Vec<Vec<Vec<u32>>> = [[[1,2], [3]], [[4,5,6]]] works
// ============================================================================
//
// LEVEL 0 (outer vector):
// ----------------------
// PASS 1:
//   - element[0] ([[1,2], [3]]): triggers its own two-pass encoding Returns size = 224 bytes (no
//     offset for nested!)
//   - element[1] ([[4,5,6]]): triggers its own two-pass encoding Returns size = 160 bytes (no
//     offset for nested!)
//
// PASS 2:
//   0x000: write offset = 32 (only because top-level)
//   0x020: write length = 2
//   0x040: write offset[0] = 64 (relative to 0x40)
//   0x060: write offset[1] = 288 (relative to 0x40)
//   0x080: write element[0] data (which does its own two-pass)
//   0x180: write element[1] data (which does its own two-pass)
//
// LEVEL 1 (middle vectors):
// ------------------------
// For element[0] = [[1,2], [3]]:
// PASS 1:
//   - [1,2]: size = 96 bytes (no offset!)
//   - [3]: size = 64 bytes (no offset!)
// PASS 2:
//   - NO offset (nested!)
//   - Writes length and offsets for inner arrays
//
// LEVEL 2 (inner vectors):
// -----------------------
// For [1,2]:
//   - Single pass (static elements)
//   - Writes: length(32) + 1(32) + 2(32) = 96 bytes (no offset!)

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::{BigEndian, LittleEndian};
    use bytes::{Bytes, BytesMut};
    /// Macro for testing a standard encode -> decode roundtrip against a hex snapshot.
    macro_rules! test_vec_encode_decode {
        (
            $test_name:ident,
            item_type = $item_type:ty,
            endian = $endian:ty,
            align = $align:expr,
            solidity = $solidity:expr,
            value = $value:expr,
            expected_hex = $expected:expr
        ) => {
            #[test]
            fn $test_name() {
                let original: Vec<$item_type> = $value;
                let mut buf = BytesMut::new();

                // Encode
                let encode_result =
                    <Vec<$item_type> as Encoder<$endian, $align, $solidity, false>>::encode(
                        &original, &mut buf, 0,
                    );
                assert!(
                    encode_result.is_ok(),
                    "Encoding failed: {:?}",
                    encode_result
                );

                // Verify encoded hex
                let encoded = buf.freeze();
                assert_eq!(hex::encode(&encoded), $expected, "Encoded data mismatch");

                // Decode and verify roundtrip
                // The type is now explicit, resolving the error.
                let decode_result =
                    <Vec<$item_type> as Encoder<$endian, $align, $solidity, false>>::decode(
                        &encoded, 0,
                    );
                assert!(
                    decode_result.is_ok(),
                    "Decoding failed: {:?}",
                    decode_result
                );
                assert_eq!(decode_result.unwrap(), original, "Decoded value mismatch");
            }
        };
    }

    // 1. Edge Cases
    test_vec_encode_decode!(
        empty_vec_non_solidity,
        item_type = u32,
        endian = BigEndian,
        align = 4,
        solidity = false,
        value = Vec::<u32>::new(),
        expected_hex = "000000000000000c00000000"
    );

    test_vec_encode_decode!(
        empty_vec_solidity,
        item_type = u32,
        endian = BigEndian,
        align = 32,
        solidity = true,
        value = Vec::<u32>::new(),
        expected_hex = "0000000000000000000000000000000000000000000000000000000000000020\
                        0000000000000000000000000000000000000000000000000000000000000000"
    );

    // 2. Basic Encoding: Types, Endianness, and Alignment
    test_vec_encode_decode!(
        vec_u32_little_endian,
        item_type = u32,
        endian = LittleEndian,
        align = 4,
        solidity = false,
        value = vec![0x12345678u32, 0x9ABCDEF0],
        expected_hex = "020000000c0000000800000078563412f0debc9a"
    );

    test_vec_encode_decode!(
        vec_u64_big_endian_with_alignment,
        item_type = u64,
        endian = BigEndian,
        align = 8,
        solidity = false,
        value = vec![1u64, 2, 3],
        expected_hex = "000000000000000300000000000000180000000000000018000000000000000100000000000000020000000000000003"
    );

    test_vec_encode_decode!(
        nested_vec_solidity,
        item_type = Vec<u32>,
        endian = BigEndian,
        align = 32,
        solidity = true,
        value = vec![vec![1u32, 2, 3], vec![4, 5]],
        expected_hex = concat!(
            // Main array header
            "0000000000000000000000000000000000000000000000000000000000000020", // offset
            "0000000000000000000000000000000000000000000000000000000000000002", // length
            "0000000000000000000000000000000000000000000000000000000000000040", // offset vec[0]
            "00000000000000000000000000000000000000000000000000000000000000c0", // offset vec[1]
            // vec[0]
            "0000000000000000000000000000000000000000000000000000000000000003", // len
            "0000000000000000000000000000000000000000000000000000000000000001",
            "0000000000000000000000000000000000000000000000000000000000000002",
            "0000000000000000000000000000000000000000000000000000000000000003",
            // vec[1]
            "0000000000000000000000000000000000000000000000000000000000000002", // len
            "0000000000000000000000000000000000000000000000000000000000000004",
            "0000000000000000000000000000000000000000000000000000000000000005"
        )
    );

    test_vec_encode_decode!(
        nested_vec_compact,
        item_type = Vec<u32>,
        endian = LittleEndian,
        align = 4,
        solidity = false,
        value = vec![
            vec![1u32, 2, 3],
            vec![4, 5],
            vec![6, 7, 8, 9, 10]
        ],
        expected_hex = concat!(
            // Main header
            "03000000", // 3 elements
            "0c000000", // offset to vec[0]
            "4c000000", // offset to vec[1]
            // vec[0] header
            "03000000", // length
            "24000000", // offset to data
            "0c000000", // data offset
            // vec[1] header
            "02000000", // length
            "30000000", // offset to data
            "08000000", // data offset
            // vec[2] header
            "05000000", // length
            "38000000", // offset to data
            "14000000", // data offset
            // data
            "01000000", // 1
            "02000000", // 2
            "03000000", // 3
            "04000000", // 4
            "05000000", // 5
            "06000000", // 6
            "07000000", // 7
            "08000000", // 8
            "09000000", // 9
            "0a000000"  // 10
        )
    );

    // 4. Manual Tests for Specific Behaviors

    #[test]
    fn vec_encoding_with_offset() {
        let original: Vec<u32> = vec![1, 2, 3, 4, 5];
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&[0xFF, 0xFF, 0xFF]); // Add some initial data

        <Vec<u32> as fluentbase_codec_old::Encoder<LittleEndian, 4, false, false>>::encode(
            &original, &mut buf, 3,
        )
        .unwrap();
        let encoded = buf.freeze();

        let decoded =
            <Vec<u32> as fluentbase_codec_old::Encoder<LittleEndian, 4, false, false>>::decode(
                &encoded, 3,
            )
            .unwrap();

        assert_eq!(original, decoded);
    }

    #[test]
    fn large_vector_roundtrip() {
        let original: Vec<u32> = (0..1000).collect();
        let mut buf = BytesMut::new();

        <Vec<u32> as Encoder<BigEndian, 4, false, false>>::encode(&original, &mut buf, 0).unwrap();
        let encoded = buf.freeze();
        let decoded =
            <Vec<u32> as Encoder<BigEndian, 4, false, false>>::decode(&encoded, 0).unwrap();

        assert_eq!(original, decoded);
    }

    #[test]
    fn decode_fails_if_buffer_is_too_small_for_header() {
        let buf = Bytes::from(vec![0x00, 0x01, 0x02, 0x03]); // Clearly too small
        let result = <Vec<u32> as Encoder<BigEndian, 4, false, false>>::decode(&buf, 0);
        assert!(result.is_err());
    }

    #[test]
    fn decode_fails_if_buffer_is_too_small_for_data() {
        // A valid header for 5 elements (5 * 4 = 20 bytes), but with no data attached.
        // Header structure (non-solidity, align=4): len (4b), offset (4b), size (4b)
        // len=5, offset=12, size=20
        let mut header = Vec::new();
        header.extend_from_slice(&5u32.to_be_bytes());
        header.extend_from_slice(&12u32.to_be_bytes());
        header.extend_from_slice(&20u32.to_be_bytes());
        let buf = Bytes::from(header);

        let result = <Vec<u32> as Encoder<BigEndian, 4, false, false>>::decode(&buf, 0);
        println!("result: {:?}", result);
        assert!(result.is_err(), "Decoding should fail when data is missing");
    }
}
