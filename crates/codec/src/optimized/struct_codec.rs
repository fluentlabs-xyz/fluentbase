// use crate::optimized::{
//     ctx::EncodingContext,
//     encoder::Encoder,
//     error::CodecError,
//     utils::{align_up, read_u32_aligned, write_u32_aligned},
// };
// use alloc::vec::Vec;
// use alloy_primitives::{Bytes, U256};
// use byteorder::ByteOrder;
// use bytes::{Buf, BufMut};
//
// #[derive(Debug, Clone, PartialEq)]
// struct ExampleInner {
//     nums: Vec<Vec<u32>>,
//     age: u32,
//     tags: Vec<u32>,
//     b: Bytes,
// }
// impl<B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, true> for ExampleInner {
//     const HEADER_SIZE: usize = {
//         let mut size = 0;
//         size += <Vec<Vec<u32>> as Encoder<B, ALIGN, true>>::HEADER_SIZE;
//         size += <u32 as Encoder<B, ALIGN, true>>::HEADER_SIZE;
//         size += <Vec<u32> as Encoder<B, ALIGN, true>>::HEADER_SIZE;
//         size += <Bytes as Encoder<B, ALIGN, true>>::HEADER_SIZE;
//
//         if <Self as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
//             size += if true { 32 } else { align_up::<ALIGN>(4) };
//         }
//         size
//     };
//
//     const IS_DYNAMIC: bool = {
//         <Vec<Vec<u32>> as Encoder<B, ALIGN, true>>::IS_DYNAMIC
//             || <u32 as Encoder<B, ALIGN, true>>::IS_DYNAMIC
//             || <Vec<u8> as Encoder<B, ALIGN, true>>::IS_DYNAMIC
//             || <Bytes as Encoder<B, ALIGN, true>>::IS_DYNAMIC
//     };
//
//     fn header_size(&self) -> usize {
//         let mut size = 0;
//         size += <Vec<Vec<u32>> as Encoder<B, ALIGN, true>>::header_size(&self.nums);
//         size += <u32 as Encoder<B, ALIGN, true>>::header_size(&self.age);
//         size += <Vec<u32> as Encoder<B, ALIGN, true>>::header_size(&self.tags);
//         size += <Bytes as Encoder<B, ALIGN, true>>::header_size(&self.b);
//
//         if <Self as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
//             size += align_up::<ALIGN>(32);
//         }
//         size
//     }
//
//     // inner
//     fn encode_header(
//         &self,
//         buf: &mut impl BufMut,
//         ctx: &mut EncodingContext,
//     ) -> Result<usize, CodecError> {
//         let mut written = 0;
//
//         if <Self as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
//             let offset = ctx.data_ptr;
//
//             // Write offset and reserve space
//             write_u32_aligned::<B, ALIGN>(buf, offset);
//             ctx.hdr_ptr += 32;
//             written += 32;
//
//             let total_size = <Self as Encoder<B, ALIGN, true>>::HEADER_SIZE
//                 + <Self as Encoder<B, ALIGN, true>>::tail_size(self, ctx)?;
//
//             if let Some(frame) = ctx.current_frame_mut() {
//                 // Теперь уже безопасно обновлять frame
//                 frame.data_ptr += total_size as u32;
//
//                 println!(
//                     "[Nested Inner] offset: {}, total reserved: {}, data_ptr updated to: {}",
//                     offset, total_size, frame.data_ptr
//                 );
//
//                 return Ok(written);
//             } else {
//                 ctx.push_frame(<Self as Encoder<B, ALIGN, true>>::HEADER_SIZE as u32);
//             }
//         }
//
//         {
//             let frame = ctx.current_frame_mut().unwrap();
//             let hdr_ptr = frame.hdr_ptr;
//
//             println!("[Inner Header] Encoding nums at hdr_ptr: {}", hdr_ptr);
//             written +=
//                 <Vec<Vec<u32>> as Encoder<B, ALIGN, true>>::encode_header(&self.nums, buf, ctx)?;
//             frame.hdr_ptr = hdr_ptr;
//
//             println!("[Inner Header] Encoding age at hdr_ptr: {}", hdr_ptr);
//             written += <u32 as Encoder<B, ALIGN, true>>::encode_header(&self.age, buf, ctx)?;
//             frame.hdr_ptr = hdr_ptr;
//
//             println!("[Inner Header] Encoding tags at hdr_ptr: {}", hdr_ptr);
//             written += <Vec<u32> as Encoder<B, ALIGN, true>>::encode_header(&self.tags, buf, ctx)?;
//             frame.hdr_ptr = hdr_ptr;
//
//             println!("[Inner Header] Encoding bytes at hdr_ptr: {}", hdr_ptr);
//             written += <Bytes as Encoder<B, ALIGN, true>>::encode_header(&self.b, buf, ctx)?;
//             frame.hdr_ptr = frame.hdr_size;
//         }
//
//         println!(
//             "[Inner Header] Final hdr_ptr: {}, data_ptr: {}",
//             ctx.hdr_ptr, ctx.data_ptr
//         );
//
//         Ok(written)
//     }
//
//     // inner
//     fn encode_tail(
//         &self,
//         buf: &mut impl BufMut,
//         ctx: &mut EncodingContext,
//     ) -> Result<usize, CodecError> {
//         let mut total_written = 0;
//
//         if ctx.current_frame().is_none() {
//             println!("[Inner Tail] Header not encoded yet, encoding now...");
//             ctx.push_frame(<Self as Encoder<B, ALIGN, true>>::HEADER_SIZE as u32);
//             <Self as Encoder<B, ALIGN, true>>::encode_header(&self, buf, ctx)?;
//         }
//
//         if <Vec<Vec<u32>> as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
//             println!("[Inner Tail] Encoding nums tail...");
//             total_written +=
//                 <Vec<Vec<u32>> as Encoder<B, ALIGN, true>>::encode_tail(&self.nums, buf, ctx)?;
//         }
//
//         if <u32 as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
//             println!("[Inner Tail] Encoding age tail...");
//             total_written += <u32 as Encoder<B, ALIGN, true>>::encode_tail(&self.age, buf, ctx)?;
//         }
//
//         if <Vec<u32> as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
//             println!("[Inner Tail] Encoding tags tail...");
//             total_written +=
//                 <Vec<u32> as Encoder<B, ALIGN, true>>::encode_tail(&self.tags, buf, ctx)?;
//         }
//
//         if <Bytes as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
//             println!("[Inner Tail] Encoding bytes tail...");
//             total_written += <Bytes as Encoder<B, ALIGN, true>>::encode_tail(&self.b, buf, ctx)?;
//         }
//
//         println!("[Inner Tail] Total written: {}", total_written);
//
//         Ok(total_written)
//     }
//
//     fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
//         let (chunk, mut cur_offset) = if <Self as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
//             if true {
//                 let tuple_relative_offset = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;
//                 (&buf.chunk()[offset + tuple_relative_offset..], 0)
//             } else {
//                 let data_offset = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;
//                 (buf.chunk(), offset + data_offset)
//             }
//         } else {
//             (buf.chunk(), offset)
//         };
//
//         let nums = <Vec<Vec<u32>> as Encoder<B, ALIGN, true>>::decode(&chunk, cur_offset)?;
//         cur_offset += <Vec<Vec<u32>> as Encoder<B, ALIGN, true>>::header_size(&nums);
//
//         let age = <u32 as Encoder<B, ALIGN, true>>::decode(&chunk, cur_offset)?;
//         cur_offset += <u32 as Encoder<B, ALIGN, true>>::header_size(&age);
//
//         let tags = <Vec<u32> as Encoder<B, ALIGN, true>>::decode(&chunk, cur_offset)?;
//         cur_offset += <Vec<u32> as Encoder<B, ALIGN, true>>::header_size(&tags);
//
//         let b = <Bytes as Encoder<B, ALIGN, true>>::decode(&chunk, cur_offset)?;
//
//         Ok(Self { nums, age, tags, b })
//     }
//
//     fn tail_size(&self, ctx: &mut EncodingContext) -> Result<usize, CodecError> {
//         let mut size = 0;
//         if <Vec<Vec<u32>> as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
//             size += <Vec<Vec<u32>> as Encoder<B, ALIGN, true>>::tail_size(&self.nums, ctx)?;
//         }
//         if <u32 as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
//             size += <u32 as Encoder<B, ALIGN, true>>::tail_size(&self.age, ctx)?;
//         }
//         if <Vec<u32> as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
//             size += <Vec<u32> as Encoder<B, ALIGN, true>>::tail_size(&self.tags, ctx)?;
//         }
//         if <Bytes as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
//             size += <Bytes as Encoder<B, ALIGN, true>>::tail_size(&self.b, ctx)?;
//         }
//         Ok(size)
//     }
// }
//
// #[derive(Debug, Clone, PartialEq)]
// struct Example {
//     inner: ExampleInner,
//     balance: U256,
//     b: Bytes,
// }
// impl<B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, true> for Example {
//     const HEADER_SIZE: usize = {
//         let mut size = 0;
//         size += <ExampleInner as Encoder<B, ALIGN, true>>::HEADER_SIZE;
//         size += <U256 as Encoder<B, ALIGN, true>>::HEADER_SIZE;
//         size += <Bytes as Encoder<B, ALIGN, true>>::HEADER_SIZE;
//
//         if <Self as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
//             size += align_up::<ALIGN>(4);
//         }
//         size
//     };
//
//     const IS_DYNAMIC: bool = {
//         <ExampleInner as Encoder<B, ALIGN, true>>::IS_DYNAMIC
//             || <U256 as Encoder<B, ALIGN, true>>::IS_DYNAMIC
//             || <Bytes as Encoder<B, ALIGN, true>>::IS_DYNAMIC
//     };
//
//     //
//     fn header_size(&self) -> usize {
//         let mut size = 0;
//         size += if <ExampleInner as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
//             align_up::<ALIGN>(4)
//         } else {
//             <ExampleInner as Encoder<B, ALIGN, true>>::HEADER_SIZE
//         };
//         size += if <U256 as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
//             align_up::<ALIGN>(4)
//         } else {
//             <U256 as Encoder<B, ALIGN, true>>::HEADER_SIZE
//         };
//         size += if <Bytes as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
//             align_up::<ALIGN>(4)
//         } else {
//             <Bytes as Encoder<B, ALIGN, true>>::HEADER_SIZE
//         };
//
//         if <Self as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
//             size += align_up::<ALIGN>(32);
//         }
//         size
//     }
//
//     // outer
//     fn encode_header(
//         &self,
//         buf: &mut impl BufMut,
//         ctx: &mut EncodingContext,
//     ) -> Result<usize, CodecError> {
//         let mut written = 0;
//
//         if <Self as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
//             let offset = ctx.data_ptr;
//
//             // Write offset and reserve space
//             write_u32_aligned::<B, ALIGN>(buf, offset);
//             ctx.hdr_ptr += 32;
//             written += 32;
//
//             if let Some(frame) = ctx.current_frame_mut() {
//                 // Nested structure
//                 let total_size = <Self as Encoder<B, ALIGN, true>>::HEADER_SIZE
//                     + <Self as Encoder<B, ALIGN, true>>::tail_size(self, ctx)?;
//
//                 frame.data_ptr += total_size as u32;
//
//                 println!(
//                     "[Nested Outer] offset: {}, total reserved: {}, data_ptr updated to: {}",
//                     offset, total_size, frame.data_ptr
//                 );
//
//                 return Ok(written);
//             } else {
//                 // Top-level structure
//                 ctx.push_frame(<Self as Encoder<B, ALIGN, true>>::HEADER_SIZE as u32);
//             }
//         }
//
//         let frame = ctx.current_frame_mut().unwrap();
//         let hdr_ptr = frame.hdr_ptr;
//
//         println!("[Outer Header] Encoding inner at hdr_ptr: {}", hdr_ptr);
//         written += <ExampleInner as Encoder<B, ALIGN, true>>::encode_header(&self.inner, buf, ctx)?;
//         frame.hdr_ptr = hdr_ptr;
//
//         println!("[Outer Header] Encoding balance at hdr_ptr: {}", hdr_ptr);
//         written += <U256 as Encoder<B, ALIGN, true>>::encode_header(&self.balance, buf, ctx)?;
//         frame.hdr_ptr = hdr_ptr;
//
//         println!("[Outer Header] Encoding bytes at hdr_ptr: {}", hdr_ptr);
//         written += <Bytes as Encoder<B, ALIGN, true>>::encode_header(&self.b, buf, ctx)?;
//         frame.hdr_ptr = frame.hdr_size;
//
//         println!(
//             "[Outer Header] Final hdr_ptr: {}, data_ptr: {}",
//             frame.hdr_ptr, frame.data_ptr
//         );
//
//         Ok(written)
//     }
//
//     // outer
//     fn encode_tail(
//         &self,
//         buf: &mut impl BufMut,
//         ctx: &mut EncodingContext,
//     ) -> Result<usize, CodecError> {
//         let mut total_written = 0;
//
//         if ctx.current_frame().is_none() {
//             println!("[Outer Tail] Header not encoded yet, encoding now...");
//             ctx.push_frame(<Self as Encoder<B, ALIGN, true>>::HEADER_SIZE as u32);
//             self.encode_header(buf, ctx)?;
//         }
//
//         println!("[Outer Tail] Encoding inner tail...");
//         total_written +=
//             <ExampleInner as Encoder<B, ALIGN, true>>::encode_tail(&self.inner, buf, ctx)?;
//
//         println!("[Outer Tail] Encoding bytes tail...");
//         total_written += <Bytes as Encoder<B, ALIGN, true>>::encode_tail(&self.b, buf, ctx)?;
//
//         println!("[Outer Tail] Total written: {}", total_written);
//
//         ctx.pop_frame();
//
//         Ok(total_written)
//     }
//
//     fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
//         let (chunk, mut cur_offset) = if <Self as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
//             if true {
//                 let tuple_relative_offset = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;
//                 (&buf.chunk()[offset + tuple_relative_offset..], 0)
//             } else {
//                 let data_offset = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;
//                 (buf.chunk(), offset + data_offset)
//             }
//         } else {
//             (buf.chunk(), offset)
//         };
//
//         let inner = <ExampleInner as Encoder<B, ALIGN, true>>::decode(&chunk, cur_offset)?;
//         cur_offset += <ExampleInner as Encoder<B, ALIGN, true>>::header_size(&inner);
//
//         let balance = <U256 as Encoder<B, ALIGN, true>>::decode(&chunk, cur_offset)?;
//         cur_offset += <U256 as Encoder<B, ALIGN, true>>::header_size(&balance);
//
//         let b = <Bytes as Encoder<B, ALIGN, true>>::decode(&chunk, cur_offset)?;
//
//         Ok(Self { inner, balance, b })
//     }
//
//     fn tail_size(&self, ctx: &mut EncodingContext) -> Result<usize, CodecError> {
//         let mut size = 0;
//         if <ExampleInner as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
//             size += <ExampleInner as Encoder<B, ALIGN, true>>::tail_size(&self.inner, ctx)?;
//         }
//         if <U256 as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
//             size += <U256 as Encoder<B, ALIGN, true>>::tail_size(&self.balance, ctx)?;
//         }
//
//         if <Bytes as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
//             size += <Bytes as Encoder<B, ALIGN, true>>::tail_size(&self.b, ctx)?;
//         }
//         Ok(size)
//     }
// }
//
// #[cfg(test)]
// mod tests {
//     use super::*;
//     use crate::optimized::{
//         struct_codec::U256,
//         utils::test_utils::{assert_codec_sol, print_encoded},
//     };
//     use alloy_primitives::Uint;
//     use byteorder::BigEndian;
//
//     #[test]
//     fn create_sol_expected() {
//         use alloy_sol_types::{sol, SolValue};
//
//         sol! {
//             struct ExampleInnerAlloy {
//                 uint32[][] nums;
//                 uint32 age;
//                 uint32[] tags;
//                 bytes b;
//             }
//
//             struct ExampleAlloy {
//                 ExampleInnerAlloy inner;
//                 uint256 balance;
//                 bytes b;
//             }
//         }
//         let inner = ExampleInnerAlloy {
//             nums: vec![vec![1, 2, 3], vec![4, 5]],
//             age: 42,
//             tags: vec![7, 8, 9],
//             b: Bytes::from("Hello World"),
//         };
//
//         let value = ExampleAlloy {
//             inner,
//             balance: U256::from(42),
//             b: Bytes::from("Hello World!"),
//         };
//         let encoded_alloy = value.abi_encode();
//         print_encoded::<BigEndian, 32>(&encoded_alloy);
//         assert!(false)
//     }
//
//     #[test]
//     fn test_example_inner_solidity_encoding() {
//         let value = ExampleInner {
//             nums: vec![vec![1, 2, 3], vec![4, 5]],
//             age: 42,
//             tags: vec![7, 8, 9],
//             b: Bytes::from("Hello World"),
//         };
//         let mut ctx = EncodingContext::default();
//
//         println!(
//             "nums tail_size: {}",
//             <Vec<Vec<u32>> as Encoder<BigEndian, 32, true>>::tail_size(&value.nums, &mut ctx)
//                 .unwrap()
//         );
//         println!(
//             "tags tail_size: {}",
//             <Vec<u32> as Encoder<BigEndian, 32, true>>::tail_size(&value.tags, &mut ctx).unwrap()
//         );
//         println!(
//             "bytes tail_size: {}",
//             <Bytes as Encoder<BigEndian, 32, true>>::tail_size(&value.b, &mut ctx).unwrap()
//         );
//
//         assert_codec_sol(
//             concat!(
//                 // ======== HEADER (offsets and static values) ========
//                 /* [0x0000] 0 */
//                 "0000000000000000000000000000000000000000000000000000000000000020", /* = 32 */
//                 /* [0x0020] 32 */
//                 "0000000000000000000000000000000000000000000000000000000000000080", /* = 128 */
//                 /* [0x0040] 64 */
//                 "000000000000000000000000000000000000000000000000000000000000002a", /* = 42 */
//                 /* [0x0060] 96 */
//                 "00000000000000000000000000000000000000000000000000000000000001c0", /* = 448 <- мы получаем 384 */
//                 /* [0x0080] 128 */
//                 "0000000000000000000000000000000000000000000000000000000000000240", /* = 576 <- мы получаем 480 */
//             ),
//             concat!(
//                 // ======== DATA SECTION (dynamic fields) ========
//                 // nums: Vec<Vec<u32>>
//                 /* [0x00a0] 160 */
//                 "0000000000000000000000000000000000000000000000000000000000000002", /* = 2 */
//                 // length nums
//                 // offset since 192
//                 /* [0x00c0] 192 */
//                 "0000000000000000000000000000000000000000000000000000000000000040", /* = 64 */
//                 // offset to nums[0]  192 + 64
//                 /* [0x00e0] 224 */
//                 "00000000000000000000000000000000000000000000000000000000000000c0", /* = 192 */
//                 // offset to nums[1] 192 + 192
//                 // nums[0]
//                 /* [0x0100] 256 */
//                 "0000000000000000000000000000000000000000000000000000000000000003", /* = 3 */
//                 // length nums[1]
//                 /* [0x0120] 288 */
//                 "0000000000000000000000000000000000000000000000000000000000000001", /* = 1 */
//                 /* [0x0140] 320 */
//                 "0000000000000000000000000000000000000000000000000000000000000002", /* = 2 */
//                 /* [0x0160] 352 */
//                 "0000000000000000000000000000000000000000000000000000000000000003", /* = 3 */
//                 // nums[1]
//                 /* [0x0180] 384 */
//                 "0000000000000000000000000000000000000000000000000000000000000002", /* = 2 */
//                 // length nums[2]
//                 /* [0x01a0] 416 */
//                 "0000000000000000000000000000000000000000000000000000000000000004", /* = 4 */
//                 /* [0x01c0] 448 */
//                 "0000000000000000000000000000000000000000000000000000000000000005", /* = 5 */
//                 // tags
//                 /* [0x01e0] 480 */
//                 "0000000000000000000000000000000000000000000000000000000000000003", /* = 3 */
//                 /* [0x0200] 512 */
//                 "0000000000000000000000000000000000000000000000000000000000000007", /* = 7 */
//                 /* [0x0220] 544 */
//                 "0000000000000000000000000000000000000000000000000000000000000008", /* = 8 */
//                 /* [0x0240] 576 */
//                 "0000000000000000000000000000000000000000000000000000000000000009", /* = 9 */
//                 // b
//                 /* [0x0260] 608 */
//                 "000000000000000000000000000000000000000000000000000000000000000b", /* = 11 */
//                 /* [0x0280] 640 */
//                 "48656c6c6f20576f726c64000000000000000000000000000000000000000000", /* = 0 */
//             ),
//             &value,
//         );
//     }
//
//     #[test]
//     fn test_example_solidity_encoding() {
//         let inner = ExampleInner {
//             nums: vec![vec![1, 2, 3], vec![4, 5]],
//             age: 42,
//             tags: vec![7, 8, 9],
//             b: Bytes::from("Hello World"),
//         };
//         let value = Example {
//             inner,
//             balance: Uint::from(42),
//             b: Bytes::from("Hello World!"),
//         };
//         assert_codec_sol(
//             concat!(
//                 // ======== HEADER Section ========
//                 // Example struct header
//                 "0000000000000000000000000000000000000000000000000000000000000020", /* [0x0000] offset to Example data = 32 */
//                 // Example fields
//                 "0000000000000000000000000000000000000000000000000000000000000060", /* [0x0020] offset to inner (ExampleInner) = 96 */
//                 "000000000000000000000000000000000000000000000000000000000000002a", /* [0x0040] balance = 42 (static value) */
//                 "00000000000000000000000000000000000000000000000000000000000002e0", /* [0x0060] offset to b (Bytes) = 736 */
//             ),
//             concat!(
//                 // ======== DATA Section (dynamic fields) ========
//                 // ExampleInner struct header (starts at 0x0080)
//                 "0000000000000000000000000000000000000000000000000000000000000080", /* [0x0080] offset to inner.nums (Vec<Vec<u32>>) = 128 */
//                 "000000000000000000000000000000000000000000000000000000000000002a", /* [0x00a0] inner.age = 42 (static value) */
//                 "00000000000000000000000000000000000000000000000000000000000001c0", /* [0x00c0] offset to inner.tags (Vec<u32>) = 448 */
//                 "0000000000000000000000000000000000000000000000000000000000000240", /* [0x00e0] offset to inner.b (Bytes) = 576 */
//                 // inner.nums: Vec<Vec<u32>> data (starts at 0x0100)
//                 "0000000000000000000000000000000000000000000000000000000000000002", /* [0x0100] inner.nums.len = 2 */
//                 "0000000000000000000000000000000000000000000000000000000000000040", /* [0x0120] offset to inner.nums[0] = 64 */
//                 "00000000000000000000000000000000000000000000000000000000000000c0", /* [0x0140] offset to inner.nums[1] = 192 */
//                 // inner.nums[0]: Vec<u32> = [1, 2, 3] (starts at 0x0160)
//                 "0000000000000000000000000000000000000000000000000000000000000003", /* [0x0160] inner.nums[0].len = 3 */
//                 "0000000000000000000000000000000000000000000000000000000000000001", /* [0x0180] inner.nums[0][0] = 1 */
//                 "0000000000000000000000000000000000000000000000000000000000000002", /* [0x01a0] inner.nums[0][1] = 2 */
//                 "0000000000000000000000000000000000000000000000000000000000000003", /* [0x01c0] inner.nums[0][2] = 3 */
//                 // inner.nums[1]: Vec<u32> = [4, 5] (starts at 0x01e0)
//                 "0000000000000000000000000000000000000000000000000000000000000002", /* [0x01e0] inner.nums[1].len = 2 */
//                 "0000000000000000000000000000000000000000000000000000000000000004", /* [0x0200] inner.nums[1][0] = 4 */
//                 "0000000000000000000000000000000000000000000000000000000000000005", /* [0x0220] inner.nums[1][1] = 5 */
//                 // inner.tags: Vec<u32> = [7, 8, 9] (starts at 0x0240)
//                 "0000000000000000000000000000000000000000000000000000000000000003", /* [0x0240] inner.tags.len = 3 */
//                 "0000000000000000000000000000000000000000000000000000000000000007", /* [0x0260] inner.tags[0] = 7 */
//                 "0000000000000000000000000000000000000000000000000000000000000008", /* [0x0280] inner.tags[1] = 8 */
//                 "0000000000000000000000000000000000000000000000000000000000000009", /* [0x02a0] inner.tags[2] = 9 */
//                 // inner.b: Bytes = "Hello World" (starts at 0x02c0)
//                 "000000000000000000000000000000000000000000000000000000000000000b", /* [0x02c0] inner.b.len = 11 */
//                 "48656c6c6f20576f726c64000000000000000000000000000000000000000000", /* [0x02e0] inner.b data = "Hello World" + padding */
//                 // b: Bytes = "Hello World!" (starts at 0x0300)
//                 "000000000000000000000000000000000000000000000000000000000000000c", /* [0x0300] b.len = 12 */
//                 "48656c6c6f20576f726c64210000000000000000000000000000000000000000", /* [0x0320] b data = "Hello World!" + padding */
//             ),
//             &value,
//         );
//     }
// }
