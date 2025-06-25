use crate::optimized::{
    encoder::{Encoder, EncodingContext},
    error::CodecError,
    utils::{align_up, read_u32_aligned, write_u32_aligned},
};
use byteorder::{BigEndian, ByteOrder};
use bytes::{Buf, BufMut};
use core::mem::size_of;
use crate::optimized::counter::ByteCounter;

/// Vec implementation for Compact ABI
/// For Compact ABI, we have:
/// - header
///   - length
///   - offset
///   - size
/// - raw body
///
/// For nested vectors, we have:
/// - header (outer vector)
/// - headers for each inner vector
/// - raw body of all inner vectors
impl<T, B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, false> for Vec<T>
where
    T: Encoder<B, ALIGN, false>,
{
    const IS_DYNAMIC: bool = true;
    const HEADER_SIZE: usize = size_of::<u32>() * 3;

    fn encode_head(
        &self,
        out: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        let word = align_up::<ALIGN>(4);
        let hdr = word * 3;

        if self.is_empty() {
            write_u32_aligned::<B, ALIGN>(out, 0);
            return Ok(word);
        }

        ctx.enter()?; // depth +1

        let mut total_body = 0usize;
        for el in self {
            let mut counter = ByteCounter::new();
            el.encode(&mut counter, ctx)?;
            total_body += el.encode(&mut counter, ctx)?; // ByteCounter, zero-alloc
        }
        
        // own header
        write_u32_aligned::<B, ALIGN>(out, self.len() as u32); // len
        write_u32_aligned::<B, ALIGN>(out, hdr as u32); // offset (=12)
        write_u32_aligned::<B, ALIGN>(out, total_body as u32); // size

        let mut bytes_written = hdr;

        // encode child elements' headers
        if T::IS_DYNAMIC {
            for el in self {
                bytes_written += el.encode_head(out, ctx)?;
            }
        }

        ctx.exit(); // depth −1
        Ok(bytes_written)
    }

    // Encode the tail of the vector, which contains the actual data.
    fn encode_tail(
        &self,
        out: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        if self.is_empty() {
            return Ok(0);
        }
        ctx.enter()?;
        let mut written = 0;
        for el in self {
            written += el.encode_tail(out, ctx)?;
        }
        ctx.exit();
        Ok(written)
    }

    fn len(&self) -> usize {
        self.len()
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let word = align_up::<ALIGN>(4);
        let len = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;

        if len == 0 {
            return Ok(Vec::new());
        }

        let off = read_u32_aligned::<B, ALIGN>(buf, offset + word)? as usize;
        let size = read_u32_aligned::<B, ALIGN>(buf, offset + word * 2)? as usize;

        if buf.remaining() < off + size {
            return Err(CodecError::BufferTooSmallMsg {
                expected: off + size,
                actual: buf.remaining(),
                message: "vector payload truncated".into(),
            });
        }

        let payload = &buf.chunk()[off..off + size];

        let mut out = Vec::with_capacity(len);
        for i in 0..len {
            let elem_off = i * align_up::<ALIGN>(T::HEADER_SIZE);
            out.push(T::decode(&payload, elem_off)?);
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
    T: Encoder<B, ALIGN, true>,
{
    const HEADER_SIZE: usize = 32; // offset pointer for top-level
    const IS_DYNAMIC: bool = true;

    fn encode(
        &self,
        buf: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        todo!("Vec<T>::encode for Compact-ABI not yet implemented");
        // let mut written = 0;
        //
        // let word_size: u32 = align_up::<ALIGN>(T::HEADER_SIZE) as u32;
        //
        // let mut default_ctx;
        // let ctx = match ctx {
        //     Some(ctx) => ctx,
        //     None => {
        //         default_ctx = EncodingContext::new();
        //         &mut default_ctx
        //     }
        // };
        //
        // // Write offset for outer container
        // if ctx.depth() == 0 {
        //     write_u32_aligned::<BigEndian, ALIGN>(buf, word_size);
        //     written += word_size;
        // }
        //
        // ctx.enter()?;
        //
        // // write data len
        // write_u32_aligned::<BigEndian, ALIGN>(buf, self.len() as u32);
        // written += word_size;
        //
        // if self.is_empty() {
        //     return Ok(written as usize);
        // }
        //
        // if T::IS_DYNAMIC {
        //     written += encode_dynamic_elements(self, buf, ctx)? as u32;
        // } else {
        //     for element in self.iter() {
        //         written += element.encode(buf, Some(ctx))? as u32;
        //     }
        // }
        //
        // ctx.exit();
        // Ok(written as usize)
    }

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

    fn encode_head(
        &self,
        out: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        todo!()
    }

    fn encode_tail(
        &self,
        out: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        todo!()
    }
}
//
// #[inline(always)]
// fn encode_dynamic_elements<T, B: ByteOrder, const ALIGN: usize>(
//     vec: &[T],
//     buf: &mut impl BufMut,
//     ctx: &mut EncodingContext,
// ) -> Result<usize, CodecError>
// where
//     T: Encoder<B, ALIGN, true>,
// {
//     let len = vec.len();
//     if len == 0 {
//         return Ok(0);
//     }
//     let word_size = align_up::<ALIGN>(1);
//
//     let mut current_offset = len * word_size;
//
//     for element in vec.iter() {
//         let size = if T::IS_DYNAMIC {
//             let mut counter = ByteCounter::new();
//             element.encode(&mut counter, Some(ctx))?;
//             counter.count()
//         } else {
//             align_up::<ALIGN>(T::HEADER_SIZE)
//         };
//
//         write_u32_aligned::<BigEndian, ALIGN>(buf, current_offset as u32);
//         current_offset += size;
//     }
//
//     // Write actual elements
//     let mut total_written = 0;
//     for element in vec.iter() {
//         let written = element.encode(buf, Some(ctx))?;
//
//         total_written += written;
//     }
//
//     Ok(len * word_size + total_written)
// }
//
// // let expected_encoded = hex::decode(concat!(
// // // Main array header
// // "03000000", // length = 3 vectors
// // "0c000000", // offset = 12 (to first element header)
// // "3C000000", // size = 60 (36 bytes headers - nested headers = nesting depth * 3 + 24 bytes
// data // (only data)) // Nested vector headers
// // // vec[0] = [1, 2]
// // "02000000", // length = 2
// // "24000000", // offset = 36 (from start of this header to its data)
// // "08000000", // size = 8 bytes
// // // vec[1] = [3]
// // "01000000", // length = 1
// // "2c000000", // offset = 44 (from start of this header to its data)
// // "04000000", // size = 4 bytes
// // // vec[2] = [4, 5, 6]
// // "03000000", // length = 3
// // "30000000", // offset = 48 (from start of this header to its data)
// // "0c000000", // size = 12 bytes
// // // Data sections
// // "01000000", // 1
// // "02000000", // 2
// // "03000000", // 3
// // "04000000", // 4
// // "05000000", // 5
// // "06000000"  // 6
// // ))
//
// // offset - depends on nesting level and previous data length
// /// let total_offsets = 0;
// /// let header_size = word_size * 3;
// /// length = v.len()
// /// offset = total_offsets + header_size;
// /// size = v.encode_data(&ConterBuffer::new(), 0) ->
// /// -> all headers+all data
// /// write length (3)
// /// write offset (12)
// /// write size (60)
//
// #[inline(always)]
// fn encode_dynamic_elements_compact<T, B: ByteOrder, const ALIGN: usize>(
//     vec: &[T],
//     buf: &mut impl BufMut,
//     ctx: &mut EncodingContext,
//     total_size: usize,
// ) -> Result<usize, CodecError>
// where
//     T: Encoder<B, ALIGN, false>,
// {
//     let word_size = align_up::<ALIGN>(4);
//     let header_size = word_size * 3;
//
//     // elements count
//
//     let mut current_offset = 0; // len + offset + size for each element
//
//     // Write headers for all elements
//     for element in vec.iter() {
//         let size = if T::IS_DYNAMIC {
//             let mut counter = ByteCounter::new();
//             element.encode_data(&mut counter, Some(ctx))?;
//             counter.count()
//         } else {
//             word_size
//         };
//
//         current_offset += header_size;
//         let actual_size = total_size - current_offset;
//         write_u32_aligned::<B, ALIGN>(buf, current_offset as u32);
//         write_u32_aligned::<B, ALIGN>(buf, total_size as u32);
//     }
//
//     // let mut total_written = 0;
//     // for element in vec.iter() {
//     //     let written = element.encode(buf, Some(ctx))?;
//     //     total_written += written;
//     // }
//
//     Ok(0)
// }
//
// #[inline(always)]
// fn encode_dynamic_elements_compact2<T, B: ByteOrder, const ALIGN: usize>(
//     vec: &[T],
//     buf: &mut impl BufMut,
//     ctx: &mut EncodingContext,
// ) -> Result<usize, CodecError>
// where
//     T: Encoder<B, ALIGN, false>,
// {
//     let len = vec.len();
//     let word_size = align_up::<ALIGN>(4);
//     let header_size = word_size * 3;
//
//     let mut elem_sizes = Vec::with_capacity(len);
//     let mut total_data_size = 0;
//
//     for element in vec.iter() {
//         let mut counter = ByteCounter::new();
//         element.encode(&mut counter, Some(ctx))?;
//         let data_size = counter.count() - header_size; // только данные
//         elem_sizes.push(data_size);
//         total_data_size += data_size;
//     }
//
//     // Общий размер = заголовки элементов + данные
//     let total_size = len * header_size + total_data_size;
//
//     // Записываем главный заголовок
//     write_u32_aligned::<B, ALIGN>(buf, len as u32);
//     write_u32_aligned::<B, ALIGN>(buf, header_size as u32);
//     write_u32_aligned::<B, ALIGN>(buf, total_size as u32); //
//
//     // Записываем заголовки элементов
//     let mut current_offset = len * header_size;
//     for (i, element) in vec.iter().enumerate() {
//         // Получаем длину элемента
//         let mut temp = Vec::new();
//         element.encode(&mut temp, Some(ctx))?;
//         let elem_len = read_u32_aligned::<B, ALIGN>(&&temp[..], 0)?;
//
//         write_u32_aligned::<B, ALIGN>(buf, elem_len);
//         write_u32_aligned::<B, ALIGN>(buf, current_offset as u32);
//         write_u32_aligned::<B, ALIGN>(buf, elem_sizes[i] as u32);
//
//         current_offset += elem_sizes[i];
//     }
//
//     // Записываем только данные элементов (без их заголовков)
//     for element in vec.iter() {
//         let mut temp = Vec::new();
//         element.encode(&mut temp, Some(ctx))?;
//         buf.put_slice(&temp[header_size..]); // пропускаем заголовок
//     }
//
//     Ok(header_size + total_size)
// }

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::LittleEndian;
    use bytes::BytesMut;

    /// Macro for testing a standard roundtrip (encode -> decode) against a hex snapshot.
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
                    <Vec<$item_type> as Encoder<$endian, $align, $solidity>>::encode(
                        &original, &mut buf, None, // None for top-level encoding
                    );
                assert!(
                    encode_result.is_ok(),
                    "Encoding failed: {:?}",
                    encode_result
                );

                // Verify encoded hex
                let encoded = buf.freeze();
                assert_eq!(
                    hex::encode(&encoded),
                    $expected,
                    "Encoded data mismatch\nActual:   {}\nExpected: {}",
                    hex::encode(&encoded),
                    $expected
                );

                // Decode and verify roundtrip
                let decode_result =
                    <Vec<$item_type> as Encoder<$endian, $align, $solidity>>::decode(
                        &encoded, 0, // Start from offset 0 for top-level
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

    // test_vec_encode_decode!(
    //     empty_vec_solidity,
    //     item_type = u32,
    //     endian = BigEndian,
    //     align = 32,
    //     solidity = true,
    //     value = Vec::<u32>::new(),
    //     expected_hex = concat!(
    //         "0000000000000000000000000000000000000000000000000000000000000020", // offset to data
    //         "0000000000000000000000000000000000000000000000000000000000000000"  // length = 0
    //     )
    // );
    //
    // test_vec_encode_decode!(
    //     test_nested_vec_solidity,
    //     item_type = Vec<u32>,
    //     endian = BigEndian,
    //     align = 32,
    //     solidity = true,
    //     value = vec![vec![1u32, 2, 3], vec![4, 5]],
    //     expected_hex = concat!(
    //         // Main array header
    //         "0000000000000000000000000000000000000000000000000000000000000020", // offset to data
    //         "0000000000000000000000000000000000000000000000000000000000000002", // length = 2
    //         "0000000000000000000000000000000000000000000000000000000000000040", // offset to
    // vec[0] (64 bytes from start of data)
    //         "00000000000000000000000000000000000000000000000000000000000000c0", // offset to
    // vec[1] (192 bytes from start of data)         // vec[0] = [1, 2, 3]
    //         "0000000000000000000000000000000000000000000000000000000000000003", // length = 3
    //         "0000000000000000000000000000000000000000000000000000000000000001", // 1
    //         "0000000000000000000000000000000000000000000000000000000000000002", // 2
    //         "0000000000000000000000000000000000000000000000000000000000000003", // 3
    //         // vec[1] = [4, 5]
    //         "0000000000000000000000000000000000000000000000000000000000000002", // length = 2
    //         "0000000000000000000000000000000000000000000000000000000000000004", // 4
    //         "0000000000000000000000000000000000000000000000000000000000000005"  // 5
    //     )
    // );
    //
    // #[test]
    // fn test_decode_simple() {
    //     let v: Vec<u32> = vec![1, 2, 3, 4, 5];
    //
    //     let mut buf = BytesMut::new();
    //     let encode_result = <Vec<u32> as Encoder<BigEndian, 32, true>>::encode(&v, &mut buf,
    // None);
    //
    //     assert!(
    //         encode_result.is_ok(),
    //         "Encoding failed: {:?}",
    //         encode_result
    //     );
    //     let encoded = buf.freeze();
    //     println!("encoded: {:?}", hex::encode(&encoded));
    //
    //     let decode_result = <Vec<u32> as Encoder<BigEndian, 32, true>>::decode(&encoded, 0);
    //     assert!(
    //         decode_result.is_ok(),
    //         "Decoding failed: {:?}",
    //         decode_result
    //     );
    //     let decoded = decode_result.unwrap();
    //     assert_eq!(decoded, v, "Decoded value mismatch");
    // }
    //
    // #[test]
    // fn test_encode_large() {
    //     let large_vec1: Vec<u32> = (0..1000).collect();
    //     let large_vec2: Vec<u32> = (1000..1200).collect();
    //     let large_vec3: Vec<u32> = (1200..1300).collect();
    //     let large_vec4: Vec<u32> = (1300..1350).collect();
    //     let large_vec5: Vec<u32> = (1300..2000).collect();
    //
    //     let v = vec![vec![large_vec1, large_vec2, large_vec3, large_vec4],vec![large_vec5]];
    //
    //     let mut buf = BytesMut::new();
    //     let encode_result =
    //         <Vec<Vec<Vec<u32>>> as Encoder<BigEndian, 32, true>>::encode(&v, &mut buf, None);
    //
    //     assert!(
    //         encode_result.is_ok(),
    //         "Encoding failed: {:?}",
    //         encode_result
    //     );
    //     let encoded = buf.freeze();
    //     println!("encoded: {:?}", hex::encode(&encoded));
    //
    //     let decode_result =
    //         <Vec<Vec<Vec<u32>>> as Encoder<BigEndian, 32, true>>::decode(&encoded, 0);
    //     assert!(
    //         decode_result.is_ok(),
    //         "Decoding failed: {:?}",
    //         decode_result
    //     );
    //     let decoded = decode_result.unwrap();
    //     assert_eq!(decoded, v, "Decoded value mismatch");
    // }
    //
    // // TODO: check do we actually need this use case
    // // TLDR: Decoding from dirty buffers with offset might be caller's responsibility to handle
    // // This test verifies decoding from a buffer with dirty/garbage data at the beginning.
    // // Current implementation expects clean buffers - decoding with offset into dirty buffer
    // fails. // This might be correct behavior: the caller should ensure proper buffer
    // alignment/cleaning // rather than the decoder handling arbitrary offsets in dirty
    // buffers. #[test]
    // #[ignore]
    // fn vec_decoding_from_dirty_buf() {
    //     let original: Vec<u32> = vec![1, 2, 3, 4, 5];
    //     let mut buf = BytesMut::new();
    //     buf.extend_from_slice(&[0xFF, 0xFF, 0xFF]); // Add some initial data
    //
    //     // Encode at top-level (None context)
    //     <Vec<u32> as Encoder<BigEndian, 32, true>>::encode(&original, &mut buf, None).unwrap();
    //     let encoded = buf.freeze();
    //
    //     eprintln!("encoded: {:?}", hex::encode(&encoded));
    //
    //     let expected_encoded = hex::decode(concat!(
    //         "ffffff",                                                           // Initial data
    //         "0000000000000000000000000000000000000000000000000000000000000020", // offset
    //         "0000000000000000000000000000000000000000000000000000000000000005", // length = 5
    //         "0000000000000000000000000000000000000000000000000000000000000001", // 1
    //         "0000000000000000000000000000000000000000000000000000000000000002", // 2
    //         "0000000000000000000000000000000000000000000000000000000000000003", // 3
    //         "0000000000000000000000000000000000000000000000000000000000000004", // 4
    //         "0000000000000000000000000000000000000000000000000000000000000005"  // 5
    //     ))
    //     .unwrap();
    //
    //     if encoded != expected_encoded {
    //         eprintln!("Encoded mismatch!");
    //         eprintln!("Expected: {}", hex::encode(&expected_encoded));
    //         eprintln!("Actual:   {}", hex::encode(&encoded));
    //     }
    //     assert_eq!(expected_encoded, encoded);
    //
    //     // Decode starting from offset 3 (after the 0xFF bytes)
    //     let decoded = <Vec<u32> as Encoder<BigEndian, 32, true>>::decode(&encoded, 3).unwrap();
    //
    //     assert_eq!(original, decoded);
    // }

    // asdf
    #[test]
    fn vec_compact_u32_simple() {
        let vec: Vec<u32> = vec![1, 2, 3, 4];

        // Encode tail
        let mut buf = BytesMut::new();
        let mut ctx = EncodingContext::new();
        let result_tail =
            <Vec<u32> as Encoder<LittleEndian, 4, false>>::encode_tail(&vec, &mut buf, &mut ctx);
        let expected_tail = hex::decode(concat!(
            "01000000", // 1
            "02000000", // 2
            "03000000", // 3
            "04000000", // 4
        ))
        .unwrap();
        let encoded_tail = buf.freeze();
        assert!(result_tail.is_ok());
        assert_eq!(encoded_tail.len(), 16); // 4 elements × 4 bytes each
        assert_eq!(hex::encode(&expected_tail), hex::encode(&encoded_tail));

        // Encode head
        let mut buf = BytesMut::new();
        let mut ctx = EncodingContext::new();
        let result_head =
            <Vec<u32> as Encoder<LittleEndian, 4, false>>::encode_head(&vec, &mut buf, &mut ctx);
        assert!(result_head.is_ok());
        let encoded_head = buf.freeze();
        let expected_head = hex::decode(concat!(
            "04000000", // length 4
            "0c000000", // offset 12 (3 header fields × 4 bytes)
            "10000000", // size = 16 (4 elements × 4 bytes)
        ))
        .unwrap();
        assert!(result_head.is_ok());
        assert_eq!(hex::encode(&expected_head), hex::encode(&encoded_head));
        assert_eq!(encoded_head.len(), 12); // 3 elements × 4 bytes each

        // encode full
        let expected: Vec<u8> = expected_head
            .iter()
            .chain(expected_tail.iter())
            .cloned()
            .collect();

        let mut buf = BytesMut::new();
        let mut ctx = EncodingContext::new();
        let res = <Vec<u32> as Encoder<LittleEndian, 4, false>>::encode(&vec, &mut buf, &mut ctx)
            .expect("full encode");

        let encoded_full = buf.freeze();

        assert_eq!(
            hex::encode(&expected),
            hex::encode(&encoded_full),
            "full (head + tail) encoding mismatch"
        );

        // Decode
        let decoded = <Vec<u32> as Encoder<LittleEndian, 4, false>>::decode(&encoded_full, 0)
            .expect("decode full");
        assert_eq!(decoded, vec, "Decoded value mismatch");
    }

    #[test]
    fn vec_compact_nested() {
        let vec: Vec<Vec<u32>> = vec![vec![1, 2], vec![3], vec![4, 5, 6]];

        // Encode tail
        let mut buf = BytesMut::new();
        let mut ctx = EncodingContext::new();
        let result_tail = <Vec<Vec<u32>> as Encoder<LittleEndian, 4, false>>::encode_tail(
            &vec, &mut buf, &mut ctx,
        );
        let expected_tail = hex::decode(concat!(
            // vec[0]
            "01000000", // 1
            "02000000", // 2
            // vec[1]
            "03000000", // 3
            // vec[2]
            "04000000", // 4
            "05000000", // 5
            "06000000"  // 6
        ))
        .unwrap();
        let encoded_tail = buf.freeze();
        assert!(result_tail.is_ok());
        assert_eq!(encoded_tail.len(), 24);
        assert_eq!(hex::encode(&expected_tail), hex::encode(&encoded_tail));

        // Encode head
        let mut buf = BytesMut::new();
        let mut ctx = EncodingContext::new();
        let result_head = <Vec<Vec<u32>> as Encoder<LittleEndian, 4, false>>::encode_head(
            &vec, &mut buf, &mut ctx,
        );
        let encoded_head = buf.freeze();
        let expected_head = hex::decode(concat!(
            // Main header
            "03000000", // length = 3
            "0c000000", // offset = 12
            "3C000000", // total size = 60
            // Nested headers
            "02000000", // len = 2
            "24000000", // offset = 36
            "08000000", // size = 8
            "01000000", // len = 1
            "2c000000", // offset = 44
            "04000000", // size = 4
            "03000000", // len = 3
            "30000000", // offset = 48
            "0c000000", // size = 12
        ))
        .unwrap();
        assert!(result_head.is_ok());
        assert_eq!(hex::encode(&expected_head), hex::encode(&encoded_head));
        assert_eq!(encoded_head.len(), 12 + 3 * 12); // top-level + 3 nested headers

        // Full encode
        let expected: Vec<u8> = expected_head
            .iter()
            .chain(expected_tail.iter())
            .cloned()
            .collect();

        let mut buf = BytesMut::new();
        let mut ctx = EncodingContext::new();
        let result =
            <Vec<Vec<u32>> as Encoder<LittleEndian, 4, false>>::encode(&vec, &mut buf, &mut ctx);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 72); // 12 + 36 + 24

        let encoded_full = buf.freeze();
        assert_eq!(
            hex::encode(&expected),
            hex::encode(&encoded_full),
            "full (head + tail) encoding mismatch"
        );

        // Decode
        let decoded = <Vec<Vec<u32>> as Encoder<LittleEndian, 4, false>>::decode(&encoded_full, 0)
            .expect("decode full");
        assert_eq!(decoded, vec, "Decoded value mismatch");
    }

    #[test]
    fn vec_compact_nested_level3() {
        // L3-значение:
        // [
        //   [ [1,2], [3] ],        // A
        //   [ [4] ]                // B
        // ]
        let test_value: Vec<Vec<Vec<u32>>> = vec![vec![vec![1, 2], vec![3]], vec![vec![4]]];

        let mut buf = BytesMut::new();
        let mut ctx = EncodingContext::new();

        // --- Encode ---------------------------------------------------------------
        let sz = <Vec<Vec<Vec<u32>>> as Encoder<LittleEndian, 4, false>>::encode(
            &test_value,
            &mut buf,
            &mut ctx,
        )
        .expect("encode");

        // --- Ожидаемое бинарное представление -------------------------------------
        let expected = hex::decode(concat!(
            // ── Main (depth-0) header ────────────────────────────────────────────
            "02000000", // len  = 2 vectors
            "0c000000", // off  = 12 (к первому header'у)
            "4c000000", // size = 76 (24 header'ов + 52 data)
            // ── L1 : vector A header ─────────────────────────────────────────────
            "02000000", // len  = 2  ([1,2], [3])
            "18000000", // off  = 24 (2 * 12)
            "24000000", // size = 36 (24 header'а L2 + 12 data)
            // ── L1 : vector B header ─────────────────────────────────────────────
            "01000000", // len  = 1  ([4])
            "0c000000", // off  = 12
            "10000000", // size = 16 (12 header + 4 data)
            // ── L2 headers inside A ----------------------------------------------
            "02000000", "0c000000", "08000000", // [1,2]
            "01000000", "0c000000", "04000000", // [3]
            // ── Data section of A -------------------------------------------------
            "01000000", "02000000", "03000000",
            // ── L2 header inside B -----------------------------------------------
            "01000000", "0c000000", "04000000", // [4]
            // ── Data section of B -------------------------------------------------
            "04000000"
        ))
        .unwrap();

        let encoded_expected = "020000000c0000004c000000020000001800000024000000010000003c00000010000000020000001800000008000000010000002000000004000000010000000200000003000000010000000c0000000400000004000000";

        // hex-представление фактического вывода
        let encoded_hex = hex::encode(buf.freeze());
        assert_eq!(
            encoded_expected, encoded_hex,
            "Depth-3 vector encoding doesn't match expected value"
        );

        // total bytes: main-hdr(12) + A-hdr(12) + B-hdr(12) + A-body(36) + B-body(16) = 88
        assert_eq!(sz, 88);
    }
}
