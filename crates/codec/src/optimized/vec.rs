use crate::optimized::{
    ctx::{EncodingContext, NodeMeta},
    encoder::Encoder,
    error::CodecError,
    utils::{align_up, read_u32_aligned},
};
use byteorder::{BigEndian, ByteOrder, LittleEndian};
use bytes::{Buf, BufMut, BytesMut};

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
    // base header size - actual header size would be // `HEADER_SIZE + len * T::HEADER_SIZE`
    const HEADER_SIZE: usize = size_of::<u32>() * 3;

    // TODO: for structs and static types we need to update implementation - to add
    // static element header size into root_hdr -
    fn build_ctx(&self, ctx: &mut EncodingContext) -> Result<(), CodecError> {
        let me = ctx.nodes.len();
        ctx.nodes.push(NodeMeta {
            len: self.len() as u32,
            tail: 0,
            total_hdr_len: 0,
        });

        let mut body = 0usize;
        let mut hdr = Self::HEADER_SIZE as u32;

        if T::IS_DYNAMIC {
            for child in self {
                let child_root = ctx.nodes.len();
                child.build_ctx(ctx)?;
                let c = ctx.nodes[child_root];

                body += c.tail as usize;
                hdr += c.total_hdr_len;
            }
        }

        if !T::IS_DYNAMIC {
            body += align_up::<ALIGN>(T::HEADER_SIZE) * self.len();
        }

        ctx.nodes[me].tail = body as u32;
        ctx.nodes[me].total_hdr_len = hdr;

        if me == 0 {
            ctx.root_hdr = hdr; // root header size
        }
        Ok(())
    }

    /// naive, single-pass header writer
    ///
    /// * writes each header once, strictly forward;
    /// * `offset` = bytes of body already reserved in the whole buffer;
    /// * recurses only if the element itself is dynamic (`T::IS_DYNAMIC`)
    fn encode_header(
        &self,
        out: &mut impl bytes::BufMut,
        ctx: &mut EncodingContext, // {nodes, index, hdr_written, body_reserved}
    ) -> Result<usize, CodecError> {
        /* take NodeMeta */
        let id = ctx.index;
        ctx.index += 1;
        let m = ctx.nodes[id]; // {len, tail, total_hdr_len}

        /* -------- off -------- */
        // check if it's a nested vectors
        let (off, size) = if T::IS_DYNAMIC {
            (T::HEADER_SIZE as u32, T::HEADER_SIZE as u32) // jump to first child-header
        } else {
            // (total_hdr − already_written_hdr - cur_off?) + already_reserved_body
            ((ctx.root_hdr - ctx.hdr_written) + ctx.body_reserved, m.tail)
        };

        /* len / off / size */
        out.put_u32_le(m.len);
        out.put_u32_le(off);
        out.put_u32_le(size); // full slice length

        ctx.hdr_written += 12;
        let mut written = 12;

        /* children */
        if T::IS_DYNAMIC {
            for child in self {
                written += child.encode_header(out, ctx)?;
            }
        }

        /* reserve body for younger siblings */
        ctx.body_reserved += m.tail;
        Ok(written)
    }

    // /* 2-nd pass – header writer (single forward walk) */
    // fn encode_header(
    //     &self,
    //     out: &mut impl BufMut,
    //     ctx: &mut Self::Ctx,
    // ) -> Result<usize, CodecError> {
    //     /* ───────── ensure root has stacks ───────── */
    //     if ctx.header_offset.is_empty() {
    //         ctx.header_offset.push(0);
    //         ctx.data_offset.push(0);
    //     }
    //     /* ----------  our own header  ---------- */
    //     let id = next_node!(ctx);
    //     let meta = ctx.nodes[id]; // { len , tail }
    //     let body_off = cur_dat!(ctx); // where our body will start
    //
    //     out.put_u32_le(meta.len); // length
    //     out.put_u32_le(body_off); // offset (rel. to parent's body)
    //     out.put_u32_le(meta.tail); // size
    //     bump_hdr!(ctx, Self::HEADER_SIZE as u32); // parent knows we've added 12 bytes
    //     let mut written = Self::HEADER_SIZE;
    //
    //     /* ----------  recurse into children  ---------- */
    //     ctx.header_offset.push(0); // new level counters
    //     ctx.data_offset.push(0);
    //
    //     for child in self {
    //         written += child.encode_header(out, ctx)?; // header of child
    //     }
    //
    //     /* after all children their bodies will be appended to ours,
    //     so add our full body size to parent’s data cursor             */
    //     ctx.header_offset.pop();
    //     bump_dat!(ctx, meta.tail);
    //     ctx.data_offset.pop();
    //
    //     Ok(written)
    // }

    fn encode_tail(&self, out: &mut impl BufMut) -> Result<usize, CodecError> {
        let mut bytes = 0;
        for el in self {
            if T::IS_DYNAMIC {
                bytes += el.encode_tail(out)?;
            } else {
                let mut ctx = EncodingContext::new();
                bytes += el.encode_header(out, &mut ctx)?;
            }
        }
        Ok(bytes)
    }

    // /// collect dynamic metadata (nested vecs)
    // fn build_ctx(&self, ctx: &mut Self::Ctx) -> Result<(), CodecError> {
    //     if self.is_empty() {
    //         return Ok(());
    //     }
    //
    //     let len = self.len();
    //
    //     let tail = if T::IS_DYNAMIC {
    //         0
    //     } else {
    //         align_up::<ALIGN>(T::HEADER_SIZE) * len
    //     };
    //
    //     ctx.nodes.push(NodeMeta {
    //         len: len as u32,
    //         tail: tail as u32,
    //     });
    //
    //     for child in self {
    //         child.build_ctx(ctx)?;
    //     }
    //
    //     Ok(())
    // }
    //
    // fn encode_header(&self, out: &mut impl BufMut, ctx: &mut Self::Ctx) -> Result<usize,
    // CodecError> {     // where data_would be written
    //     let data_offset = ctx.data_offset;
    //     let header_offset = ctx.header_offset;
    //
    //     let mut written = 0;
    //
    //     // Write length, offset and size
    //     let len = ctx.nodes.len();
    //     // offset to the actual data since current position
    //     let offset_to_data = data_offset - header_offset;
    //
    //     // size of the whole data
    //     let size = ctx.nodes.iter().map(|n| n.tail as usize).sum::<usize>() + self.len() *
    // T::HEADER_SIZE;
    //
    //     write_u32_aligned::<B, ALIGN>(out, len as u32);
    //     written += 4;
    //     write_u32_aligned::<B, ALIGN>(out, offset_to_data as u32);
    //     written += 4;
    //     write_u32_aligned::<B, ALIGN>(out, size as u32);
    //
    //     for el in self.iter() {
    //         el.encode_header(out, ctx)?;
    //     }
    //
    //     Ok(written)
    //
    // }

    // fn encode_tail(&self, out: &mut impl BufMut) -> Result<usize, CodecError> {
    //     if self.is_empty() {
    //         return Ok(0);
    //     }
    //     let mut bytes = 0;
    //     for el in self {
    //         bytes += el.encode_tail(out)?;
    //     }
    //
    //     Ok(bytes)
    // }

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

    fn encode(&self, buf: &mut impl BufMut) -> Result<usize, CodecError> {
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

    fn encode_tail(&self, out: &mut impl BufMut) -> Result<usize, CodecError> {
        todo!()
    }

    fn encode_header(
        &self,
        out: &mut impl BufMut,
        ctx: &mut Self::Ctx,
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
    fn vec_compact_u32_simple_tail() {
        let vec: Vec<u32> = vec![1, 2, 3, 4];

        // // Header-pass (для статической Vec<u32> он ничего не пишет):
        // let mut ctx = EncodingContext::default();
        // <Vec<u32> as Encoder<LittleEndian, 4, false>>::build_ctx(&vec, &mut ctx).unwrap();

        // Tail-pass: здесь появятся сами u32
        let mut buf = BytesMut::new();
        <Vec<u32> as Encoder<LittleEndian, 4, false>>::encode_tail(&vec, &mut buf).unwrap();

        assert_eq!(hex::encode(&buf), "01000000020000000300000004000000");
    }

    #[test]
    fn vec_compact_u32_simple_head() {
        let vec: Vec<u32> = vec![1, 2, 3, 4];

        // Encode head
        let mut buf = BytesMut::new();
        let mut ctx = EncodingContext::new();
        let result_ctx = <Vec<u32> as Encoder<LittleEndian, 4, false>>::build_ctx(&vec, &mut ctx);
        assert!(result_ctx.is_ok());
        println!("ctx: {:?}", ctx);

        let result_head =
            <Vec<u32> as Encoder<LittleEndian, 4, false>>::encode_header(&vec, &mut buf, &mut ctx);
        let expected_head = hex::decode(concat!(
            "04000000", // length 4
            "0c000000", // offset 12 (3 header fields × 4 bytes)
            "10000000", // size = 16 (4 elements × 4 bytes)
        ))
        .unwrap();

        let encoded_head = buf.freeze();
        assert_eq!(hex::encode(&expected_head), hex::encode(&encoded_head));
        assert!(result_head.is_ok());
        assert_eq!(encoded_head.len(), 12); // 3 elements × 4 bytes each
    }

    #[test]
    fn vec_compact_build_ctx() {
        // build ctx

        let v: Vec<u32> = vec![1, 2, 3, 4];
        let mut ctx = EncodingContext::new();
        let res = <Vec<u32> as Encoder<LittleEndian, 4, false>>::build_ctx(&v, &mut ctx);
        assert!(res.is_ok(), "build_ctx failed: {:?}", res);
        let expected_nodes: SmallVec<[NodeMeta; 8]> = smallvec![NodeMeta {
            len: 4,
            tail: 16,
            total_hdr_len: 12
        },];
        assert_eq!(expected_nodes, ctx.nodes, "Context nodes mismatch");

        // build ctx
        let v: Vec<Vec<u32>> = vec![vec![1, 2, 3], vec![4, 5]];
        let mut ctx = EncodingContext::new();
        let res = <Vec<Vec<u32>> as Encoder<LittleEndian, 4, false>>::build_ctx(&v, &mut ctx);
        assert!(res.is_ok(), "build_ctx failed: {:?}", res);
        let expected_nodes: SmallVec<[NodeMeta; 8]> = smallvec![
            NodeMeta {
                len: 2,
                tail: 20,
                total_hdr_len: 36
            }, // root
            NodeMeta {
                len: 3,
                tail: 12,
                total_hdr_len: 12
            }, // vec![1, 2, 3]
            NodeMeta {
                len: 2,
                tail: 8,
                total_hdr_len: 12
            }, // vec![4, 5]
        ];

        assert_eq!(expected_nodes, ctx.nodes, "Context nodes mismatch");
        assert_eq!(expected_nodes, ctx.nodes, "Context nodes mismatch nested 2");

        // build ctx
        let v: Vec<Vec<Vec<u32>>> = vec![vec![vec![1, 2], vec![3], vec![4, 5, 6]]];
        let mut ctx = EncodingContext::new();
        let res = <Vec<Vec<Vec<u32>>> as Encoder<LittleEndian, 4, false>>::build_ctx(&v, &mut ctx);
        assert!(res.is_ok(), "build_ctx failed: {:?}", res);
        let expected_nodes: SmallVec<[NodeMeta; 8]> = smallvec![
            NodeMeta {
                len: 1,
                tail: 24,
                total_hdr_len: 60
            }, // root Vec<Vec<Vec<u32>>>
            NodeMeta {
                len: 3,
                tail: 24,
                total_hdr_len: 48
            }, // first Vec<Vec<u32>>
            NodeMeta {
                len: 2,
                tail: 8,
                total_hdr_len: 12
            }, // vec![1, 2]
            NodeMeta {
                len: 1,
                tail: 4,
                total_hdr_len: 12
            }, // vec![3]
            NodeMeta {
                len: 3,
                tail: 12,
                total_hdr_len: 12
            }, // vec![4, 5, 6]
        ];

        assert_eq!(expected_nodes, ctx.nodes, "Context nodes mismatch nested 3");
    }

    #[test]
    fn vec_compact_write_header() {
        // empty
        let v: Vec<u32> = vec![];
        let mut ctx = EncodingContext::new();
        let res = <Vec<u32> as Encoder<LittleEndian, 4, false>>::build_ctx(&v, &mut ctx);
        assert!(res.is_ok(), "build_ctx failed: {:?}", res);
        println!("ctx: {:?}", ctx);
        let mut buf = BytesMut::new();
        let written =
            <Vec<u32> as Encoder<LittleEndian, 4, false>>::encode_header(&v, &mut buf, &mut ctx)
                .expect("encode_header must succeed");

        let encoded = buf.freeze();
        let expected = concat!(
        "00000000", // length 4
        "0c000000", // offset 12 (3 header fields × 4 bytes)
        "00000000"  // size = 16 (4 elements × 4 bytes)
        );
        assert_eq!(hex::encode(&encoded), expected, "Empty header encoding mismatch");

        // simple
        let v: Vec<u32> = vec![1, 2, 3, 4];
        // prepare ctx
        let mut ctx = EncodingContext::new();
        <Vec<u32> as Encoder<LittleEndian, 4, false>>::build_ctx(&v, &mut ctx)
            .expect("build_ctx must succeed");

        println!("ctx: {:?}", ctx);

        let mut buf = BytesMut::new();
        let written =
            <Vec<u32> as Encoder<LittleEndian, 4, false>>::encode_header(&v, &mut buf, &mut ctx)
                .expect("encode_header must succeed");

        let encoded = buf.freeze();
        let expected = concat!(
        "04000000", // length 4
        "0c000000", // offset 12 (3 header fields × 4 bytes)
        "10000000"  // size = 16 (4 elements × 4 bytes)
        );
        assert_eq!(hex::encode(&encoded), expected, "Header encoding mismatch");

        // nested
        let v: Vec<Vec<u32>> = vec![vec![1, 2, 3], vec![4, 5]];
        let mut ctx = EncodingContext::new();
        let res = <Vec<Vec<u32>> as Encoder<LittleEndian, 4, false>>::build_ctx(&v, &mut ctx);
        assert!(res.is_ok(), "build_ctx failed: {:?}", res);

        let expected_nodes: SmallVec<[NodeMeta; 8]> = smallvec![
            NodeMeta {
                len: 2,
                tail: 20,
                total_hdr_len: 36
            }, // root
            NodeMeta {
                len: 3,
                tail: 12,
                total_hdr_len: 12
            }, // vec![1, 2, 3]
            NodeMeta {
                len: 2,
                tail: 8,
                total_hdr_len: 12
            }, // vec![4, 5]
        ];
        assert_eq!(expected_nodes, ctx.nodes, "Context nodes mismatch nested 2");
        println!("ctx: {:?}", ctx);

        let mut buf = BytesMut::new();
        let written = <Vec<Vec<u32>> as Encoder<LittleEndian, 4, false>>::encode_header(
            &v, &mut buf, &mut ctx,
        )
        .expect("encode_header must succeed");
        assert_eq!(written, 36, "Header size should be 36 bytes (3 headers)");

        let encoded = buf.freeze();
        // root Vec [[1,2,3], [4,5]]
        let expected_encode = concat!(
            /* ── root header ─────────────────────────────── */
            "02000000", /* len(root)  = 2 */
            "0c000000", /* off(root)  = 12 → jump to 1-st child hdr */
            "0c000000", /* size(root) = 12 → size of 1-st child hdr */
            /* ── child-0 header (vec![1,2,3]) ─────────────── */
            "03000000", /* len       = 3 */
            "18000000", /* off       = 24 → jump to its data */
            "0c000000", /* size      = 12 (=3×4) */
            /* ── child-1 header (vec![4,5]) ────────────────── */
            "02000000", /* len       = 2 */
            "18000000", /* off       = 24 → jump past child-0 data */
            "08000000"  /* size      = 8  (=2×4) */
        );
        assert_eq!(expected_encode, hex::encode(encoded), "Header encoding mismatch");
    }
}

//
//
// #[test]
// fn vec_compact_u32_simple() {
//     let vec: Vec<u32> = vec![1, 2, 3, 4];
//
//     // Encode tail
//     let mut buf = BytesMut::new();
//     let mut ctx = EncodingContext::new();
//     let result_tail =
//         <Vec<u32> as Encoder<LittleEndian, 4, false>>::encode_tail(&vec, &mut buf);
//     let expected_tail = hex::decode(concat!(
//     "01000000", // 1
//     "02000000", // 2
//     "03000000", // 3
//     "04000000", // 4
//     ))
//         .unwrap();
//     let encoded_tail = buf.freeze();
//     assert_eq!(hex::encode(&expected_tail), hex::encode(&encoded_tail));
//     assert!(result_tail.is_ok());
//     assert_eq!(encoded_tail.len(), 16); // 4 elements × 4 bytes each
//
//     // Encode head
//     let mut buf = BytesMut::new();
//     let mut ctx = EncodingContext::new();
//     let result_ctx = <Vec<u32> as Encoder<LittleEndian, 4, false>>::build_ctx(&vec, &mut ctx);
//     assert!(result_ctx.is_ok());
//
//     let result_head =
//         <Vec<u32> as Encoder<LittleEndian, 4, false>>::encode_header(&vec, &mut buf, &mut ctx);
//     let expected_head = hex::decode(concat!(
//     "04000000", // length 4
//     "0c000000", // offset 12 (3 header fields × 4 bytes)
//     "10000000", // size = 16 (4 elements × 4 bytes)
//     ))
//         .unwrap();
//
//     let encoded_head = buf.freeze();
//     assert!(result_head.is_ok());
//     assert_eq!(encoded_head.len(), 12); // 3 elements × 4 bytes each
//     assert_eq!(hex::encode(&expected_head), hex::encode(&encoded_head));
//
//     // encode full
//     let expected: Vec<u8> = expected_head
//         .iter()
//         .chain(expected_tail.iter())
//         .cloned()
//         .collect();
//
//     let mut buf = BytesMut::new();
//     let mut ctx = EncodingContext::new();
//     let res = <Vec<u32> as Encoder<LittleEndian, 4, false>>::encode(&vec, &mut buf)
//         .expect("full encode");
//
//     let encoded_full = buf.freeze();
//
//     assert_eq!(
//         hex::encode(&expected),
//         hex::encode(&encoded_full),
//         "full (head + tail) encoding mismatch"
//     );
//
//     // Decode
//     let decoded = <Vec<u32> as Encoder<LittleEndian, 4, false>>::decode(&encoded_full, 0)
//         .expect("decode full");
//     assert_eq!(decoded, vec, "Decoded value mismatch");
// }
