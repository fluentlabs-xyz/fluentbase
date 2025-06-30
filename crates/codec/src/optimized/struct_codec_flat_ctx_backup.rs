use crate::optimized::{
    ctx::EncodingContext,
    encoder::Encoder,
    error::CodecError,
    utils::{align_up, read_u32_aligned, write_u32_aligned},
};
use alloc::vec::Vec;
use alloy_primitives::{Bytes, U256};
use byteorder::ByteOrder;
use bytes::{Buf, BufMut};

#[derive(Debug, Clone, PartialEq)]
struct ExampleInner {
    nums: Vec<Vec<u32>>,
    age: u32,
    tags: Vec<u32>,
    b: Bytes,
}
impl<B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, true> for ExampleInner {
    const HEADER_SIZE: usize = {
        let mut size = 0;
        size += <Vec<Vec<u32>> as Encoder<B, ALIGN, true>>::HEADER_SIZE;
        size += <u32 as Encoder<B, ALIGN, true>>::HEADER_SIZE;
        size += <Vec<u32> as Encoder<B, ALIGN, true>>::HEADER_SIZE;
        size += <Bytes as Encoder<B, ALIGN, true>>::HEADER_SIZE;

        if <Self as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            size += if true { 32 } else { align_up::<ALIGN>(4) };
        }
        size
    };

    const IS_DYNAMIC: bool = {
        <Vec<Vec<u32>> as Encoder<B, ALIGN, true>>::IS_DYNAMIC
            || <u32 as Encoder<B, ALIGN, true>>::IS_DYNAMIC
            || <Vec<u8> as Encoder<B, ALIGN, true>>::IS_DYNAMIC
            || <Bytes as Encoder<B, ALIGN, true>>::IS_DYNAMIC
    };

    fn header_size(&self) -> usize {
        let mut size = 0;
        size += <Vec<Vec<u32>> as Encoder<B, ALIGN, true>>::header_size(&self.nums);
        size += <u32 as Encoder<B, ALIGN, true>>::header_size(&self.age);
        size += <Vec<u32> as Encoder<B, ALIGN, true>>::header_size(&self.tags);
        size += <Bytes as Encoder<B, ALIGN, true>>::header_size(&self.b);

        if <Self as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            size += align_up::<ALIGN>(32);
        }
        size
    }

    // inner
    fn encode_header(
        &self,
        buf: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        if ctx.depth > 0 {
            
        }
        let mut written = 0;
        
        let hdr_ptr = ctx.hdr_ptr;

        // Sequentially encode headers of each field, resetting hdr_ptr each time to initial
        // position
        println!("[Inner Header] Encoding nums at hdr_ptr: {}", hdr_ptr);
        written += <Vec<Vec<u32>> as Encoder<B, ALIGN, true>>::encode_header(&self.nums, buf, ctx)?;
        ctx.hdr_ptr = hdr_ptr;

        println!("[Inner Header] Encoding age at hdr_ptr: {}", hdr_ptr);
        written += <u32 as Encoder<B, ALIGN, true>>::encode_header(&self.age, buf, ctx)?;
        ctx.hdr_ptr = hdr_ptr;

        println!("[Inner Header] Encoding tags at hdr_ptr: {}", hdr_ptr);
        written += <Vec<u32> as Encoder<B, ALIGN, true>>::encode_header(&self.tags, buf, ctx)?;
        ctx.hdr_ptr = hdr_ptr;

        println!("[Inner Header] Encoding bytes at hdr_ptr: {}", hdr_ptr);
        written += <Bytes as Encoder<B, ALIGN, true>>::encode_header(&self.b, buf, ctx)?;
        ctx.hdr_ptr = ctx.hdr_size;

        // Mark header as encoded to prevent duplicate processing
        ctx.header_encoded = true;

        // Final state logging
        println!(
            "[Inner Header] Final hdr_ptr: {}, data_ptr: {}",
            ctx.hdr_ptr, ctx.data_ptr
        );

        Ok(written)
    }

    // inner
    fn encode_tail(
        &self,
        buf: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        let mut total_written = 0;

        if !ctx.header_encoded {
            println!("[Inner Tail] Header not encoded yet, encoding now...");
            <ExampleInner as Encoder<B, ALIGN, true>>::encode_header(self, buf, ctx)?;
        }

        if <Vec<Vec<u32>> as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            println!("[Inner Tail] Encoding nums tail...");
            total_written +=
                <Vec<Vec<u32>> as Encoder<B, ALIGN, true>>::encode_tail(&self.nums, buf, ctx)?;
        }

        if <u32 as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            println!("[Inner Tail] Encoding age tail...");
            total_written += <u32 as Encoder<B, ALIGN, true>>::encode_tail(&self.age, buf, ctx)?;
        }

        if <Vec<u32> as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            println!("[Inner Tail] Encoding tags tail...");
            total_written +=
                <Vec<u32> as Encoder<B, ALIGN, true>>::encode_tail(&self.tags, buf, ctx)?;
        }

        if <Bytes as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            println!("[Inner Tail] Encoding bytes tail...");
            total_written += <Bytes as Encoder<B, ALIGN, true>>::encode_tail(&self.b, buf, ctx)?;
        }

        println!("[Inner Tail] Total written: {}", total_written);

        Ok(total_written)
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let (chunk, mut cur_offset) = if <Self as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            if true {
                let tuple_relative_offset = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;
                (&buf.chunk()[offset + tuple_relative_offset..], 0)
            } else {
                let data_offset = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;
                (buf.chunk(), offset + data_offset)
            }
        } else {
            (buf.chunk(), offset)
        };

        let nums = <Vec<Vec<u32>> as Encoder<B, ALIGN, true>>::decode(&chunk, cur_offset)?;
        cur_offset += <Vec<Vec<u32>> as Encoder<B, ALIGN, true>>::header_size(&nums);

        let age = <u32 as Encoder<B, ALIGN, true>>::decode(&chunk, cur_offset)?;
        cur_offset += <u32 as Encoder<B, ALIGN, true>>::header_size(&age);

        let tags = <Vec<u32> as Encoder<B, ALIGN, true>>::decode(&chunk, cur_offset)?;
        cur_offset += <Vec<u32> as Encoder<B, ALIGN, true>>::header_size(&tags);

        let b = <Bytes as Encoder<B, ALIGN, true>>::decode(&chunk, cur_offset)?;

        Ok(Self { nums, age, tags, b })
    }

    fn tail_size(&self, ctx: &mut EncodingContext) -> Result<usize, CodecError> {
        let mut size = 0;
        if <Vec<Vec<u32>> as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            size += <Vec<Vec<u32>> as Encoder<B, ALIGN, true>>::tail_size(&self.nums, ctx)?;
        }
        if <u32 as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            size += <u32 as Encoder<B, ALIGN, true>>::tail_size(&self.age, ctx)?;
        }
        if <Vec<u32> as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            size += <Vec<u32> as Encoder<B, ALIGN, true>>::tail_size(&self.tags, ctx)?;
        }
        if <Bytes as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            size += <Bytes as Encoder<B, ALIGN, true>>::tail_size(&self.b, ctx)?;
        }
        Ok(size)
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Example {
    inner: ExampleInner,
    balance: U256,
}
impl<B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, true> for Example {
    const HEADER_SIZE: usize = {
        let mut size = 0;
        size += <ExampleInner as Encoder<B, ALIGN, true>>::HEADER_SIZE;
        size += <U256 as Encoder<B, ALIGN, true>>::HEADER_SIZE;

        if <Self as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            size += align_up::<ALIGN>(4);
        }
        size
    };

    const IS_DYNAMIC: bool = {
        <ExampleInner as Encoder<B, ALIGN, true>>::IS_DYNAMIC
            || <U256 as Encoder<B, ALIGN, true>>::IS_DYNAMIC
    };

    //
    fn header_size(&self) -> usize {
        let mut size = 0;
        size += if <ExampleInner as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            align_up::<ALIGN>(4)
        } else {
            <ExampleInner as Encoder<B, ALIGN, true>>::HEADER_SIZE
        };
        size += if <U256 as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            align_up::<ALIGN>(4)
        } else {
            <U256 as Encoder<B, ALIGN, true>>::HEADER_SIZE
        };

        if <Self as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            size += align_up::<ALIGN>(32);
        }
        size
    }

    // outer
    fn encode_header(
        &self,
        buf: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        let mut written = 0;

        let depth = ctx.depth;

        // Determine if structure is dynamic and compute offset accordingly
        if <Self as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            let offset = if depth == 0 {
                // Top-level structures always start at offset 32
                32u32
            } else {
                // Nested structures calculate relative offset based on header and data pointers
                (ctx.hdr_size - ctx.hdr_ptr + ctx.data_ptr) as u32
            };

            // Log computed offset and context for debugging
            println!(
                "[Inner Header] depth: {}, hdr_ptr: {}, data_ptr: {}, offset: {}",
                ctx.depth, ctx.hdr_ptr, ctx.data_ptr, offset
            );

            // Write calculated offset and update header pointer
            write_u32_aligned::<B, ALIGN>(buf, offset);
            ctx.hdr_ptr += 32;
            written += 32;

            if ctx.depth > 0 {
                let hdr_size = <Self as Encoder<B, ALIGN, true>>::HEADER_SIZE as u32;
                let tail_size = <Self as Encoder<B, ALIGN, true>>::tail_size(self, ctx)? as u32;

                println!(
                    "[Nested Inner] hdr_size: {}; tail_size: {};",
                    hdr_size, tail_size
                );
                // For nested structures, immediately advance data_ptr by the total size
                // HEADER_SIZE + tail_size represents complete space reservation
                let total_size = hdr_size + tail_size - ctx.hdr_ptr;

                // Log total reserved size and the updated data pointer
                println!(
                    "[Nested Inner] total_size (header+tail adjusted): {}, updated data_ptr: {}",
                    total_size,
                    ctx.data_ptr + total_size
                );

                ctx.data_ptr += total_size;
                return Ok(written);
            }

            // Increment depth for nested header encoding logic
            ctx.depth += 1;
        }

        let hdr_ptr = ctx.hdr_ptr;

        // Sequentially encode headers of each field, resetting hdr_ptr to initial position after
        // each field
        println!("[Outer Header] Encoding inner at hdr_ptr: {}", hdr_ptr);
        if <ExampleInner as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            let offset = if ctx.depth == 0 {
                32u32
            } else {
                (ctx.hdr_size - ctx.hdr_ptr + ctx.data_ptr) as u32
            };
            let size = <ExampleInner as Encoder<B, ALIGN, true>>::tail_size(
                &self.inner,
                &mut EncodingContext::new(),
            )? as u32;
            write_u32_aligned::<B, ALIGN>(buf, offset);
            ctx.data_ptr += size;
        } else {
            written +=
                <ExampleInner as Encoder<B, ALIGN, true>>::encode_header(&self.inner, buf, ctx)?;
            ctx.hdr_ptr = hdr_ptr;
        }

        println!("[Outer Header] Encoding balance at hdr_ptr: {}", hdr_ptr);
        written += <U256 as Encoder<B, ALIGN, true>>::encode_header(&self.balance, buf, ctx)?;
        ctx.hdr_ptr = hdr_ptr;

        // println!("[Outer Header] Encoding bytes at hdr_ptr: {}", hdr_ptr);
        // written += <Bytes as Encoder<B, ALIGN, true>>::encode_header(&self.b, buf, ctx)?;
        // ctx.hdr_ptr = ctx.hdr_size;

        // Mark header as encoded to avoid duplicate encoding
        ctx.header_encoded = true;

        // Final state logging
        println!(
            "[Outer Header] Final hdr_ptr: {}, data_ptr: {}",
            ctx.hdr_ptr, ctx.data_ptr
        );

        Ok(written)
    }

    // outer
    fn encode_tail(
        &self,
        buf: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        let mut total_written = 0;

        if <ExampleInner as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            println!("[Example Tail] Encoding inner tail...");
            total_written +=
                <ExampleInner as Encoder<B, ALIGN, true>>::encode_header(&self.inner, buf, ctx)?;
            total_written +=
                <ExampleInner as Encoder<B, ALIGN, true>>::encode_tail(&self.inner, buf, ctx)?;
        }

        if <U256 as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            println!("[Example Tail] Encoding balance tail...");
            total_written +=
                <U256 as Encoder<B, ALIGN, true>>::encode_tail(&self.balance, buf, ctx)?;
        }

        println!("[Example Tail] Total written: {}", total_written);

        Ok(total_written)
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let (chunk, mut cur_offset) = if <Self as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            if true {
                let tuple_relative_offset = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;
                (&buf.chunk()[offset + tuple_relative_offset..], 0)
            } else {
                let data_offset = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;
                (buf.chunk(), offset + data_offset)
            }
        } else {
            (buf.chunk(), offset)
        };

        let inner = <ExampleInner as Encoder<B, ALIGN, true>>::decode(&chunk, cur_offset)?;
        cur_offset += <ExampleInner as Encoder<B, ALIGN, true>>::header_size(&inner);

        let balance = <U256 as Encoder<B, ALIGN, true>>::decode(&chunk, cur_offset)?;
        cur_offset += <U256 as Encoder<B, ALIGN, true>>::header_size(&balance);

        Ok(Self { inner, balance })
    }

    fn tail_size(&self, ctx: &mut EncodingContext) -> Result<usize, CodecError> {
        let mut size = 0;
        if <ExampleInner as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            size += <ExampleInner as Encoder<B, ALIGN, true>>::tail_size(&self.inner, ctx)?;
        }
        if <U256 as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            size += <U256 as Encoder<B, ALIGN, true>>::tail_size(&self.balance, ctx)?;
        }

        Ok(size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::optimized::utils::test_utils::{assert_codec_sol, print_encoded};
    use alloy_primitives::Uint;
    use byteorder::BigEndian;

    #[test]
    fn create_sol_expected() {
        use alloy_sol_types::{sol, SolValue};

        sol! {
            struct ExampleInnerAlloy {
                uint32[][] nums;
                uint32 age;
                uint32[] tags;
                bytes b;
            }

            struct ExampleAlloy {
                ExampleInnerAlloy inner;
                uint256 balance;
            }
        }
        let inner = ExampleInnerAlloy {
            nums: vec![vec![1, 2, 3], vec![4, 5]],
            age: 42,
            tags: vec![7, 8, 9],
            b: Bytes::from("Hello World"),
        };

        let value = ExampleAlloy {
            inner,
            balance: U256::from(42),
        };
        let encoded_alloy = value.abi_encode();
        print_encoded::<BigEndian, 32>(&encoded_alloy);
        assert!(false)
    }

    #[test]
    fn test_example_inner_solidity_encoding() {
        let value = ExampleInner {
            nums: vec![vec![1, 2, 3], vec![4, 5]],
            age: 42,
            tags: vec![7, 8, 9],
            b: Bytes::from("Hello World"),
        };
        let mut ctx = EncodingContext::default();

        println!(
            "nums tail_size: {}",
            <Vec<Vec<u32>> as Encoder<BigEndian, 32, true>>::tail_size(&value.nums, &mut ctx)
                .unwrap()
        );
        println!(
            "tags tail_size: {}",
            <Vec<u32> as Encoder<BigEndian, 32, true>>::tail_size(&value.tags, &mut ctx).unwrap()
        );
        println!(
            "bytes tail_size: {}",
            <Bytes as Encoder<BigEndian, 32, true>>::tail_size(&value.b, &mut ctx).unwrap()
        );

        assert_codec_sol(
            concat!(
                // ======== HEADER (offsets and static values) ========
                /* [0x0000] 0 */
                "0000000000000000000000000000000000000000000000000000000000000020", /* = 32 */
                /* [0x0020] 32 */
                "0000000000000000000000000000000000000000000000000000000000000080", /* = 128 */
                /* [0x0040] 64 */
                "000000000000000000000000000000000000000000000000000000000000002a", /* = 42 */
                /* [0x0060] 96 */
                "00000000000000000000000000000000000000000000000000000000000001c0", /* = 448 <- мы получаем 384 */
                /* [0x0080] 128 */
                "0000000000000000000000000000000000000000000000000000000000000240", /* = 576 <- мы получаем 480 */
            ),
            concat!(
                // ======== DATA SECTION (dynamic fields) ========
                // nums: Vec<Vec<u32>>
                /* [0x00a0] 160 */
                "0000000000000000000000000000000000000000000000000000000000000002", /* = 2 */
                // length nums
                // offset since 192
                /* [0x00c0] 192 */
                "0000000000000000000000000000000000000000000000000000000000000040", /* = 64 */
                // offset to nums[0]  192 + 64
                /* [0x00e0] 224 */
                "00000000000000000000000000000000000000000000000000000000000000c0", /* = 192 */
                // offset to nums[1] 192 + 192
                // nums[0]
                /* [0x0100] 256 */
                "0000000000000000000000000000000000000000000000000000000000000003", /* = 3 */
                // length nums[1]
                /* [0x0120] 288 */
                "0000000000000000000000000000000000000000000000000000000000000001", /* = 1 */
                /* [0x0140] 320 */
                "0000000000000000000000000000000000000000000000000000000000000002", /* = 2 */
                /* [0x0160] 352 */
                "0000000000000000000000000000000000000000000000000000000000000003", /* = 3 */
                // nums[1]
                /* [0x0180] 384 */
                "0000000000000000000000000000000000000000000000000000000000000002", /* = 2 */
                // length nums[2]
                /* [0x01a0] 416 */
                "0000000000000000000000000000000000000000000000000000000000000004", /* = 4 */
                /* [0x01c0] 448 */
                "0000000000000000000000000000000000000000000000000000000000000005", /* = 5 */
                // tags
                /* [0x01e0] 480 */
                "0000000000000000000000000000000000000000000000000000000000000003", /* = 3 */
                /* [0x0200] 512 */
                "0000000000000000000000000000000000000000000000000000000000000007", /* = 7 */
                /* [0x0220] 544 */
                "0000000000000000000000000000000000000000000000000000000000000008", /* = 8 */
                /* [0x0240] 576 */
                "0000000000000000000000000000000000000000000000000000000000000009", /* = 9 */
                // b
                /* [0x0260] 608 */
                "000000000000000000000000000000000000000000000000000000000000000b", /* = 11 */
                /* [0x0280] 640 */
                "48656c6c6f20576f726c64000000000000000000000000000000000000000000", /* = 0 */
            ),
            &value,
        );
    }

    #[test]
    fn test_example_solidity_encoding() {
        let inner = ExampleInner {
            nums: vec![vec![1, 2, 3], vec![4, 5]],
            age: 42,
            tags: vec![7, 8, 9],
            b: Bytes::from("Hello World"),
        };
        let value = Example {
            inner,
            balance: Uint::from(42),
        };
        assert_codec_sol(
            concat!(
                "0000000000000000000000000000000000000000000000000000000000000020", /* [0x0000] 0 = 32 */
                "0000000000000000000000000000000000000000000000000000000000000040", /* [0x0020] 32 = 64 */
                "000000000000000000000000000000000000000000000000000000000000002a", /* [0x0040] 64 = 42 */
            ),
            concat!(
                "0000000000000000000000000000000000000000000000000000000000000080", /* [0x0060] 96 = 128 */
                "000000000000000000000000000000000000000000000000000000000000002a", /* [0x0080] 128 = 42 */
                "00000000000000000000000000000000000000000000000000000000000001c0", /* [0x00a0] 160 = 448 */
                "0000000000000000000000000000000000000000000000000000000000000240", /* [0x00c0] 192 = 576 */
                "0000000000000000000000000000000000000000000000000000000000000002", /* [0x00e0] 224 = 2 */
                "0000000000000000000000000000000000000000000000000000000000000040", /* [0x0100] 256 = 64 */
                "00000000000000000000000000000000000000000000000000000000000000c0", /* [0x0120] 288 = 192 */
                "0000000000000000000000000000000000000000000000000000000000000003", /* [0x0140] 320 = 3 */
                "0000000000000000000000000000000000000000000000000000000000000001", /* [0x0160] 352 = 1 */
                "0000000000000000000000000000000000000000000000000000000000000002", /* [0x0180] 384 = 2 */
                "0000000000000000000000000000000000000000000000000000000000000003", /* [0x01a0] 416 = 3 */
                "0000000000000000000000000000000000000000000000000000000000000002", /* [0x01c0] 448 = 2 */
                "0000000000000000000000000000000000000000000000000000000000000004", /* [0x01e0] 480 = 4 */
                "0000000000000000000000000000000000000000000000000000000000000005", /* [0x0200] 512 = 5 */
                "0000000000000000000000000000000000000000000000000000000000000003", /* [0x0220] 544 = 3 */
                "0000000000000000000000000000000000000000000000000000000000000007", /* [0x0240] 576 = 7 */
                "0000000000000000000000000000000000000000000000000000000000000008", /* [0x0260] 608 = 8 */
                "0000000000000000000000000000000000000000000000000000000000000009", /* [0x0280] 640 = 9 */
                "000000000000000000000000000000000000000000000000000000000000000b", /* [0x02a0] 672 = 11 */
                "48656c6c6f20576f726c64000000000000000000000000000000000000000000", /* [0x02c0] 704 = 0 */
            ),
            &value,
        );
    }
}
