use crate::optimized::{
    ctx::{EncodingContext, NodeMeta},
    encoder::Encoder,
    error::CodecError,
    utils::{align_up, read_u32_aligned},
};
use byteorder::{BigEndian, ByteOrder};
use bytes::{Buf, BufMut};

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
/// Other static fields if we in the struct
///   
/// ------
/// - raw body of all inner vectors
impl<T, B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, false> for Vec<T>
where
    T: Encoder<B, ALIGN, false, Ctx = EncodingContext>,

{
    type Ctx = EncodingContext;
    const IS_DYNAMIC: bool = true;
    const HEADER_SIZE: usize = core::mem::size_of::<u32>() * 3;

    /// collect dynamic metadata (nested vecs)
    fn build_ctx(&self, ctx: &mut Self::Ctx) -> Result<(), CodecError> {
        if self.is_empty() {
            return Ok(());
        }

        let len = self.len();

        let tail = if T::IS_DYNAMIC {
            0
        } else {
            align_up::<ALIGN>(T::HEADER_SIZE) * len
        };

        ctx.nodes.push(NodeMeta {
            len: len as u32,
            tail: tail as u32,
        });
        
        for child in self {
            child.build_ctx(ctx)?;
        }

        Ok(())
    }

    fn encode_header(&self, out: &mut impl BufMut, ctx: &Self::Ctx) -> Result<usize, CodecError> {
        todo!()
    }


    fn encode_tail(&self, out: &mut impl BufMut) -> Result<usize, CodecError> {
        if self.is_empty() {
            return Ok(0);
        }
        let mut bytes = 0;
        for el in self {
            bytes += el.encode_tail(out)?;
        }

        Ok(bytes)
    }

    #[inline]
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
    type Ctx = EncodingContext;
    const HEADER_SIZE: usize = 32; // offset pointer for top-level
    const IS_DYNAMIC: bool = true;

    fn encode(
        &self,
        buf: &mut impl BufMut,

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

    fn build_ctx(&self, ctx: &mut EncodingContext) -> Result<(), CodecError> {
        todo!()
    }

    fn encode_tail(
        &self,
        out: &mut impl BufMut,
    ) -> Result<usize, CodecError> {
        todo!()
    }

    fn encode_header(&self, out: &mut impl BufMut, ctx: &Self::Ctx) -> Result<usize, CodecError> {
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
    use crate::optimized::utils::write_u32_aligned;
    use byteorder::LittleEndian;
    use bytes::BytesMut;
    use smallvec::{smallvec, SmallVec};

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

    #[test]
    fn vec_compact_u32_simple() {
        let vec: Vec<u32> = vec![1, 2, 3, 4];

        // Encode tail
        let mut buf = BytesMut::new();
        let mut ctx = EncodingContext::new();
        let result_tail =
            <Vec<u32> as Encoder<LittleEndian, 4, false>>::encode_tail(&vec, &mut buf);
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
        let result_head = <Vec<u32> as Encoder<LittleEndian, 4, false>>::build_ctx(&vec, &mut ctx);
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
        let res = <Vec<u32> as Encoder<LittleEndian, 4, false>>::encode(&vec, &mut buf)
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
    // asdf
    #[test]
    fn vec_compact_build_ctx() {
        let v: Vec<Vec<Vec<u32>>> = vec![vec![vec![1, 2], vec![3], vec![4, 5, 6]]];
        
        // build ctx
        let mut ctx = EncodingContext::new();
        let res = <Vec<Vec<Vec<u32>>> as Encoder<LittleEndian, 4, false>>::build_ctx(&v, &mut
        ctx); assert!(res.is_ok(), "build_ctx failed: {:?}", res);
        let expected_nodes: SmallVec<[NodeMeta; 8]> = smallvec![
            NodeMeta { len: 1, tail: 0 },  // root Vec<Vec<Vec<u32>>>
            NodeMeta { len: 3, tail: 0 },  // first Vec<Vec<u32>>
            NodeMeta { len: 2, tail: 8 },  // vec![1, 2]
            NodeMeta { len: 1, tail: 4 },  // vec![3]
            NodeMeta { len: 3, tail: 12 }, // vec![4, 5, 6]
        ];
        
        assert_eq!(ctx.nodes, expected_nodes, "Context nodes mismatch");

        let v: Vec<Vec<u32>> = vec![vec![1, 2, 3], vec![4, 5]];
        let mut ctx = EncodingContext::new();
        let res = <Vec<Vec<u32>> as Encoder<LittleEndian, 4, false>>::build_ctx(&v, &mut ctx);
        assert!(res.is_ok(), "build_ctx failed: {:?}", res);
        let expected_nodes: SmallVec<[NodeMeta; 8]> = smallvec![
            NodeMeta { len: 2, tail: 0 },  // root
            NodeMeta { len: 3, tail: 12 }, // vec![1, 2, 3]
            NodeMeta { len: 2, tail: 8 },  // vec![4, 5]
        ];
        assert_eq!(ctx.nodes, expected_nodes, "Context nodes mismatch");

        // let mut buf = BytesMut::new();
        // let written = finalize_ctx::<LittleEndian, 4>(&mut ctx, &mut buf);
        // 
        // let encoded = buf.freeze();
        // println!("{:?}", encoded);
        // let expected = concat!(
        //     /* root Vec<Vec<u32>> */
        //     "03000000", // len = 3
        //     "0c000000", // offset = 12 (сразу за root-header’ом)
        //     "4c000000", // size  = 36 + 40 = 76(все headers детей + их данные)
        //     /* vec![1, 2, 3] */
        //     "03000000", // len = 3
        //     "24000000", // offset = 36 (hdr_block 3*12 = 36)
        //     "0c000000", // size  = 12
        //     /* vec![4, 5] */
        //     "02000000", // len = 2
        //     "30000000", // offset = 48 (36 + 12 пред. tail)
        //     "08000000", // size  = 8
        //     /* vec![6, 7, 8, 9, 10] */
        //     "05000000", // len = 5
        //     "38000000", // offset = 56 (36 + 12 + 8)
        //     "14000000", // size  = 20
        // );
        // 
        // assert_eq!(expected, hex::encode(&encoded))
    }
    // version 3

    /// Emit all compact-ABI headers for the dynamic containers recorded in `ctx.nodes`
    ///
    /// Returns the total number of header bytes written.
    fn finalize_ctx<B: ByteOrder, const ALIGN: usize>(
        ctx: &EncodingContext,
        buf: &mut impl BufMut,
    ) -> usize {
        // size of one header word and total header size (3 words)
        let word = ALIGN.max(4);
        let hdr = 3 * word;

        // data for the first container starts immediately after its L₀ headers
        let mut data_offset = hdr * ctx.nodes[0].len as usize;
        let mut written = 0;

        // stack of “remaining children” for each open container
        let mut remaining: SmallVec<[usize; 16]> = SmallVec::new();
        remaining.push(ctx.nodes[0].len as usize);

        for node in &ctx.nodes {
            // pop completed levels
            while remaining.last().copied() == Some(0) {
                remaining.pop();
            }
            // consume one child from the current level
            if let Some(top) = remaining.last_mut() {
                *top -= 1;
            }

            if node.len > 0 {
                // dynamic container → emit [len, offset, size]
                write_u32_aligned::<B, ALIGN>(buf, node.len);
                write_u32_aligned::<B, ALIGN>(buf, data_offset as u32);
                write_u32_aligned::<B, ALIGN>(buf, node.tail);
                written += hdr;

                // this container’s data comes after its headers
                data_offset += node.tail as usize;
                // now expect `len` child-headers
                remaining.push(node.len as usize);
            } else {
                // static leaf → just advance data_offset by its fixed tail
                data_offset += node.tail as usize;
            }
        }

        written
    }

    // version 2
    // use crate::optimized::utils::{align_up, write_u32_aligned};
    // use byteorder::ByteOrder;
    // use bytes::BufMut;

    //
    // /// pass-1: линейно пишем заголовки `[len, offset, size]`
    // /// Возвращает общее количество записанных байт
    // fn emit_headers<B: ByteOrder, const ALIGN: usize>(
    //     nodes: &[NodeMeta],
    //     out: &mut impl BufMut,
    // ) -> Result<u32, CodecError> {
    //     let hdr_size = (align_up::<ALIGN>(4) * 3) as u32;
    //     let mut written = 0;
    //     let mut current_offset = 0;
    //
    //     // Сначала вычисляем общий размер заголовков
    //     let total_headers_size = nodes.iter().filter(|n| n.len > 0).count() as u32 * hdr_size;
    //
    //     // Теперь пишем заголовки с правильными offset'ами
    //     for (i, n) in nodes.iter().enumerate() {
    //         if n.len == 0 {
    //             continue;
    //         } // листья не пишут header
    //
    //         // offset указывает на начало данных узла после всех заголовков
    //         let node_offset = total_headers_size + current_offset;
    //
    //         write_u32_aligned::<B, ALIGN>(out, n.len as u32);
    //         write_u32_aligned::<B, ALIGN>(out, node_offset);
    //         write_u32_aligned::<B, ALIGN>(out, n.tail);
    //
    //         written += hdr_size;
    //         current_offset += n.tail; // смещаем на размер данных узла
    //     }
    //
    //     Ok(written)
    // }
    //
    // /// публичная zero-alloc обёртка
    // pub fn finalize_ctx<B: ByteOrder, const ALIGN: usize>(
    //     ctx: &mut EncodingContext,
    //     out: &mut impl BufMut,
    // ) -> Result<u32, CodecError> {
    //     if ctx.nodes.is_empty() {
    //         return Ok(0);
    //     }
    //     println!("ctx.nodes before: {:?}", ctx.nodes);
    //
    //     let mut tail_size: u32 = 0;
    //     let mut header_size: u32 = 0;
    //
    //     for i in (0..ctx.nodes.len()).rev() {
    //         tail_size += ctx.nodes[i].tail as u32;
    //         header_size += 12;
    //         // top level
    //         if ctx.nodes[i].tail == 0 {
    //             // Внутренний узел - суммируем tail непосредственных детей
    //             ctx.nodes[i].tail = tail_size;
    //         }
    //     }
    //
    //     println!("ctx.nodes after: {:?}", ctx.nodes);
    //
    //     // pass-1: пишем заголовки
    //     emit_headers::<B, ALIGN>(&ctx.nodes, out)
    // }

    // version 1 - errors in indexes
    // use bytes::BufMut;
    // use byteorder::ByteOrder;
    // use crate::optimized::utils::{align_up, write_u32_aligned};
    //
    // /// pass-0: рекурсивно вычисляем `tail` для каждого узла
    // ///
    // /// возвращает ( next_index_за_поддеревом , tail_байты_поддерева )
    // fn calc_tail<const ALIGN: usize>(
    //     nodes: &mut [NodeMeta],
    //     idx: usize,
    // ) -> (usize, usize) {
    //     let len  = nodes[idx].len as usize;
    //     if len == 0 {
    //         // лист: tail уже записан build_ctx
    //         let tail = nodes[idx].tail as usize;
    //         return (idx + 1, tail);
    //     }
    //
    //     let mut cur   = idx + 1;
    //     let mut total = 0;
    //     for _ in 0..len {
    //         let (next, child_tail) = calc_tail::<ALIGN>(nodes, cur);
    //         total += child_tail;
    //         cur    = next;
    //     }
    //     nodes[idx].tail = total as u32; // запись in-place
    //     (cur, total)
    // }
    //
    // /// pass-1: линейно пишем заголовки `[len, offset, size]`
    // fn emit_headers<B: ByteOrder, const ALIGN: usize>(
    //     nodes: &[NodeMeta],
    //     out:   &mut impl BufMut,
    // ) -> u32 {
    //     let hdr  = (align_up::<ALIGN>(4) * 3) as u32;
    //     let mut written = 0;
    //
    //     for n in nodes {
    //         if n.len == 0 { continue; }            // листья не пишут header
    //         write_u32_aligned::<B, ALIGN>(out, n.len  as u32);
    //         write_u32_aligned::<B, ALIGN>(out, hdr * n.len); // offset = hdr*len
    //         write_u32_aligned::<B, ALIGN>(out, n.tail);
    //         written += hdr;
    //     }
    //     written
    // }
    //
    // /// публичная zero-alloc обёртка
    // pub fn finalize_ctx<B: ByteOrder, const ALIGN: usize>(
    //     ctx: &mut EncodingContext,
    //     out: &mut impl BufMut,
    // ) -> Result<u32, CodecError> {
    //     if ctx.nodes.is_empty() { return Ok(0); }
    //
    //     // pass-0: нормализуем tail’ы
    //     calc_tail::<ALIGN>(&mut ctx.nodes, 0);
    //
    //     // pass-1: пишем заголовки
    //     Ok(emit_headers::<B, ALIGN>(&ctx.nodes, out))
    // }
}
