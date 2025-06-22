use crate::{
    alloc::string::ToString,
    bytes_codec::{read_bytes, read_bytes_header},
    encoder::{align_up, read_u32_aligned, write_u32_aligned, Encoder},
    error::{CodecError, DecodingError},
};
use alloc::{fmt::Debug, vec::Vec};
use byteorder::ByteOrder;
use bytes::{Buf, BufMut, BytesMut};
use core::mem::MaybeUninit;

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
        // offset is not used in the BufMut - it has it's own offset, so we can use this offset to
        // track top level only
        let is_top_level = offset == 0;
        let start_remaining = buf.remaining_mut();

        // for top level offset always 32
        if is_top_level {
            write_u32_aligned::<B, ALIGN>(buf, 32);
        }
        // length
        write_u32_aligned::<B, ALIGN>(buf, self.len() as u32);

        // Encode based on element type
        if T::IS_DYNAMIC {
            encode_dynamic_elements::<T, B, ALIGN>(self, buf)?;
        } else {
            // Static elements: sequential write
            for item in self.iter() {
                item.encode(buf, 1)?;
            }
        }

        // Return number of bytes written
        Ok(start_remaining - buf.remaining_mut())
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        // Проверяем, что в буфере достаточно данных
        if buf.remaining() < offset + 32 {
            return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                expected: offset + 32,
                found: buf.remaining(),
                msg: "buf too small".to_string(),
            }));
        }

        // Читаем первое значение по offset
        let first_value = read_u32_aligned::<B, ALIGN>(buf, offset)?;

        // Определяем, это offset pointer или length
        // Если это 32 (0x20), то это offset pointer к данным массива
        let (data_offset, is_top_level) = if first_value == 32 {
            // Top-level: первое значение это offset, данные начинаются с offset + 32
            (offset + 32, true)
        } else {
            // Nested: первое значение это length, данные начинаются с offset
            (offset, false)
        };

        // Читаем длину массива
        let length = read_u32_aligned::<B, ALIGN>(buf, data_offset)? as usize;

        if length == 0 {
            return Ok(Vec::new());
        }

        let mut result = Vec::with_capacity(length);

        if T::IS_DYNAMIC {
            // Dynamic elements: читаем через offset pointers
            let offsets_start = data_offset + 32; // После поля length

            for i in 0..length {
                let offset_position = offsets_start + i * 32;
                let element_offset = read_u32_aligned::<B, ALIGN>(buf, offset_position)? as usize;
                // Offset относительно начала зоны offsets
                let absolute_offset = offsets_start + element_offset;
                result.push(T::decode(buf, absolute_offset)?);
            }
        } else {
            // Static elements: читаем последовательно
            let mut current_offset = data_offset + 32; // После поля length
            let element_size = align_up::<ALIGN>(T::HEADER_SIZE);

            for _ in 0..length {
                result.push(T::decode(buf, current_offset)?);
                current_offset += element_size;
            }
        }

        Ok(result)
    }
    fn partial_decode(buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        let is_top_level = offset != 0;

        let data_offset = if is_top_level {
            offset + read_u32_aligned::<B, ALIGN>(buf, offset)? as usize
        } else {
            offset
        };

        let length = read_u32_aligned::<B, ALIGN>(buf, data_offset)? as usize;

        // Conservative size estimate
        let element_size = if T::IS_DYNAMIC {
            32 // minimum for offset
        } else {
            align_up::<ALIGN>(T::HEADER_SIZE)
        };

        let header_size = if is_top_level { 32 } else { 0 };
        let total_size = header_size + 32 + length * element_size;

        Ok((offset, total_size))
    }

    fn size_hint(&self) -> usize {
        // Для обратной совместимости возвращаем полный размер (с offset header)
        self.size_hint_nested(false)
    }

    #[inline]
    fn size_hint_nested(&self, is_nested: bool) -> usize {
        if self.is_empty() {
            // Пустой вектор: offset (если top-level) + length
            return if is_nested { 32 } else { 64 };
        }

        // Базовый размер: offset (если top-level) + length
        let base_size = if is_nested { 32 } else { 64 };

        let content_size = if T::IS_DYNAMIC {
            // Для динамических элементов: offsets + данные элементов
            self.len() * 32
                + self
                    .iter()
                    .map(|item| item.size_hint_nested(true))
                    .sum::<usize>()
        } else {
            // Для статических элементов: просто данные
            self.len() * align_up::<ALIGN>(T::HEADER_SIZE)
        };

        align_up::<ALIGN>(base_size + content_size)
    }
}

// Single unified function for dynamic elements
#[inline(never)]
fn encode_dynamic_elements<T, B: ByteOrder, const ALIGN: usize>(
    vec: &[T],
    buf: &mut impl BufMut,
) -> Result<(), CodecError>
where
    T: Encoder<B, ALIGN, true, false>,
{
    const SMALL_VEC_SIZE: usize = 16;

    if vec.is_empty() {
        return Ok(());
    }

    if vec.len() <= SMALL_VEC_SIZE {
        // Stack buffer для маленьких векторов
        let mut sizes = [0usize; SMALL_VEC_SIZE];
        let mut current_offset = vec.len() * 32;

        // Считаем размеры nested элементов и пишем оффсеты
        for (i, item) in vec.iter().enumerate() {
            write_u32_aligned::<B, ALIGN>(buf, current_offset as u32);
            sizes[i] = item.size_hint_nested(true); // true = nested
            current_offset += sizes[i];
        }

        // Пишем данные
        for item in vec.iter() {
            item.encode(buf, 1)?; // offset != 0 означает nested
        }
    } else {
        // Heap allocation для больших векторов
        let sizes: Vec<usize> = vec.iter().map(|item| item.size_hint_nested(true)).collect();

        let mut current_offset = vec.len() * 32;
        for &size in &sizes {
            write_u32_aligned::<B, ALIGN>(buf, current_offset as u32);
            current_offset += size;
        }

        for item in vec.iter() {
            item.encode(buf, 1)?;
        }
    }

    Ok(())
}
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

    // test_vec_encode_decode!(
    //     nested_vec_compact,
    //     item_type = Vec<u32>,
    //     endian = LittleEndian,
    //     align = 4,
    //     solidity = false,
    //     value = vec![
    //         vec![1u32, 2, 3],
    //         vec![4, 5],
    //         vec![6, 7, 8, 9, 10]
    //     ],
    //     expected_hex = concat!(
    //         // Main header
    //         "03000000", // 3 elements
    //         "0c000000", // offset to vec[0]
    //         "4c000000", // offset to vec[1]
    //         // vec[0] header
    //         "03000000", // length
    //         "24000000", // offset to data
    //         "0c000000", // data offset
    //         // vec[1] header
    //         "02000000", // length
    //         "30000000", // offset to data
    //         "08000000", // data offset
    //         // vec[2] header
    //         "05000000", // length
    //         "38000000", // offset to data
    //         "14000000", // data offset
    //         // data
    //         "01000000", // 1
    //         "02000000", // 2
    //         "03000000", // 3
    //         "04000000", // 4
    //         "05000000", // 5
    //         "06000000", // 6
    //         "07000000", // 7
    //         "08000000", // 8
    //         "09000000", // 9
    //         "0a000000"  // 10
    //     )
    // );

    // 4. Manual Tests for Specific Behaviors

    // #[test]
    // fn test_alloy_vec_u32_dirty_buffer() {
    //     use alloy_sol_types::{sol_data::*, SolType, SolValue};
    //     // Original data
    //     let original: Vec<u32> = vec![1, 2, 3, 4, 5];

    //     // Create dirty buffer
    //     let mut buf = BytesMut::new();
    //     buf.extend_from_slice(&[0xFF, 0xFF, 0xFF]);

    //     // Encode using Alloy
    //     let encoded = original.abi_encode();
    //     buf.extend_from_slice(&encoded);

    //     // Print the buffer for reference
    //     println!("Buffer contents: {:?}", hex::encode(&buf));
    //     println!("Buffer length: {}", buf.len());
    //     assert_eq!(true, false);

    //     // Decode (skipping the garbage bytes)
    //     // let encoded_slice = &buf[3..]; // Skip first 3 bytes
    //     // let decoded: Vec<u32> = Vec::<u32>::abi_decode(encoded_slice, true).unwrap();

    //     // assert_eq!(original, decoded);
    // }
    #[test]
    fn vec_encoding_with_offset() {
        let original: Vec<u32> = vec![1, 2, 3, 4, 5];
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&[0xFF, 0xFF, 0xFF]); // Add some initial data

        <Vec<u32> as Encoder<BigEndian, 32, true, false>>::encode(&original, &mut buf, 0).unwrap();
        let encoded = buf.freeze();

        eprintln!("encoded: {:?}", hex::encode(&encoded));

        let expected_encoded = hex::decode(concat!(
            "ffffff",
            "0000000000000000000000000000000000000000000000000000000000000020",
            "0000000000000000000000000000000000000000000000000000000000000005",
            "0000000000000000000000000000000000000000000000000000000000000001",
            "0000000000000000000000000000000000000000000000000000000000000002",
            "0000000000000000000000000000000000000000000000000000000000000003",
            "0000000000000000000000000000000000000000000000000000000000000004",
            "0000000000000000000000000000000000000000000000000000000000000005"
        ))
        .unwrap();

        if encoded != expected_encoded {
            eprintln!("Encoded mismatch!");
            eprintln!("Expected: {}", hex::encode(&expected_encoded));
            eprintln!("Actual:   {}", hex::encode(&encoded));
        }
        assert_eq!(expected_encoded, encoded);

        let decoded =
            <Vec<u32> as Encoder<BigEndian, 32, true, false>>::decode(&encoded, 3).unwrap();

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

    // #[test]
    // fn decode_fails_if_buffer_is_too_small_for_data() {
    //     // A valid header for 5 elements (5 * 4 = 20 bytes), but with no data attached.
    //     // Header structure (non-solidity, align=4): len (4b), offset (4b), size (4b)
    //     // len=5, offset=12, size=20
    //     let mut header = Vec::new();
    //     header.extend_from_slice(&5u32.to_be_bytes());
    //     header.extend_from_slice(&12u32.to_be_bytes());
    //     header.extend_from_slice(&20u32.to_be_bytes());
    //     let buf = Bytes::from(header);

    //     let result = <Vec<u32> as Encoder<BigEndian, 4, false, false>>::decode(&buf, 0);
    //     println!("result: {:?}", result);
    //     assert!(result.is_err(), "Decoding should fail when data is missing");
    // }
}
