use crate::optimized::{
    ctx::EncodingContext,
    encoder::Encoder,
    error::CodecError,
    utils::{align_up, read_u32_aligned, write_u32_aligned},
};
use alloc::vec::Vec;
use alloy_primitives::{Bytes, U256};
use byteorder::{BigEndian, ByteOrder};
use bytes::{Buf, BufMut};
use core::fmt::Debug;

pub fn encode_field_header<T, B, const ALIGN: usize>(
    field: &T,
    buf: &mut impl BufMut,
    ctx: &mut EncodingContext,
) -> Result<usize, CodecError>
where
    T: Encoder<B, ALIGN, true> + Debug,
    B: ByteOrder,
{
    let mut written = 0;
    println!("[encode_field_header] for {:?} ", &field);

    // for dynamic values we need to write only offset and advance data_ptr
    if T::IS_DYNAMIC {
        let offset = if ctx.depth == 0 {
            // Top-level: полный размер всех заголовков + уже записанные данные - текущий hdr_ptr
            ctx.hdr_size + ctx.data_ptr - ctx.hdr_ptr
        } else {
            // Nested: offset просто data_ptr (фактически, текущий размер уже написанных данных)
            ctx.data_ptr - ctx.hdr_ptr
        }; 
        // let offset = ctx.hdr_size + ctx.data_ptr - ctx.hdr_ptr;
        println!(
            "[encode_field_header] depth: {}, hdr_size: {} + data_ptr: {} - hdr_ptr: {} == offset: {}",
            ctx.depth, ctx.hdr_size, ctx.data_ptr, ctx.hdr_ptr, offset
        );

        write_u32_aligned::<B, ALIGN>(buf, offset);
        println!("[encode_field_header] writing offset {:?}", offset);
        written += 32;
        // set headher encoded flag - to avoid extra heading writing (inside actual
        // T::encode_header)
        ctx.header_encoded = true;

        let tail_size = T::tail_size(field, &mut EncodingContext::default())?;
        println!("[encode_field_header] tail_size: {:?}", tail_size);
        println!(
            "[encode_field_header] header_size: {:?}",
            T::header_size(field)
        );
        println!("[encode_field_header] HEADER_SIZE: {:?}", T::HEADER_SIZE);
        // move data ptr tail + header - offset length (we already add it to the header)
        // what if we on the top level?
        ctx.data_ptr += (tail_size + T::HEADER_SIZE - 32) as u32;
        // move hdr ptr by 1 word
        ctx.hdr_ptr += 32;
    } else {
        // add static values header as is
        written += T::encode_header(field, buf, ctx)?;
        ctx.hdr_ptr += written as u32;

        // for static values we don't need to advance data ptr
    }

    Ok(written)
}

pub fn encode_field_tail<T, B, const ALIGN: usize>(
    field: &T,
    buf: &mut impl BufMut,
    ctx: &mut EncodingContext,
) -> Result<usize, CodecError>
where
    T: Encoder<B, ALIGN, true> + Debug,
    B: ByteOrder,
{
    println!("[encode_field_tail] for {:?} ", &field);

    // if value dynamic - at this point we already write offset, so we need to substract it?
    // data_ptr -
    if T::IS_DYNAMIC {
        println!("[encode_field_tail] T::IS_DYNAMIC, creating local ctx");
        println!("[encode_field_tail] global ctx {:?} ", &ctx);
        let mut local_ctx = EncodingContext {
            // I guess this is not correct -
            hdr_ptr: 32, // since we already write this offset
            data_ptr: field.header_size() as u32,
            hdr_size: T::HEADER_SIZE as u32,
            depth: ctx.depth + 1,
            header_encoded: ctx.header_encoded,
        };
        println!("[encode_field_tail] local ctx {:?} ", &local_ctx);


        let mut written = 0;
        println!("[encode_field_tail] encoding nested field header...");
        written += field.encode_header(buf, &mut local_ctx)?;
        println!("[encode_field_tail] written: {:?}", &written);
        println!("[encode_field_tail] encoding nested field tail...");
        written += field.encode_tail(buf, &mut local_ctx)?;
        println!("[encode_field_tail] written: {:?}", &written);

        println!("[encode_field_tail] preparing to update ctx.data_ptr. global: {:?}, local: (header+data): {:?}",ctx.data_ptr, &local_ctx);
        ctx.data_ptr +=
            align_up::<ALIGN>((local_ctx.hdr_size + local_ctx.data_ptr) as usize) as u32;

        println!(
            "[encode_field_tail] Completed nested field encoding: total_written={}, updated data_ptr={}",
            written, ctx.data_ptr
        );

        Ok(written)
    } else {
        Ok(0)
    }
}

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

    fn encode_header(
        &self,
        buf: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        let mut written = 0;

        if ctx.depth == 0 && <Self as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            // Top-level container offset always 32
            write_u32_aligned::<B, ALIGN>(buf, 32u32);
            ctx.hdr_ptr += 32;
            written += 32;
        }

        let initial_hdr_ptr = ctx.hdr_ptr;

        written += encode_field_header::<_, B, ALIGN>(&self.nums, buf, ctx)?;
        ctx.hdr_ptr = initial_hdr_ptr;

        written += encode_field_header::<_, B, ALIGN>(&self.age, buf, ctx)?;
        ctx.hdr_ptr = initial_hdr_ptr;

        written += encode_field_header::<_, B, ALIGN>(&self.tags, buf, ctx)?;
        ctx.hdr_ptr = initial_hdr_ptr;

        written += encode_field_header::<_, B, ALIGN>(&self.b, buf, ctx)?;
        ctx.hdr_ptr = ctx.hdr_size;

        Ok(written)
    }

    fn encode_tail(
        &self,
        buf: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        let mut written = 0;

        written += encode_field_tail::<_, B, ALIGN>(&self.nums, buf, ctx)?;
        written += encode_field_tail::<_, B, ALIGN>(&self.age, buf, ctx)?; // for static just skip
        written += encode_field_tail::<_, B, ALIGN>(&self.tags, buf, ctx)?;
        written += encode_field_tail::<_, B, ALIGN>(&self.b, buf, ctx)?;

        Ok(written)
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
        cur_offset += if <Vec<Vec<u32>> as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            ALIGN
        } else {
            <Vec<Vec<u32>> as Encoder<B, ALIGN, true>>::HEADER_SIZE
        };

        let age = <u32 as Encoder<B, ALIGN, true>>::decode(&chunk, cur_offset)?;
        cur_offset += if <u32 as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            ALIGN
        } else {
            <u32 as Encoder<B, ALIGN, true>>::HEADER_SIZE
        };

        let tags = <Vec<u32> as Encoder<B, ALIGN, true>>::decode(&chunk, cur_offset)?;
        cur_offset += if <Vec<u32> as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            ALIGN
        } else {
            <Vec<u32> as Encoder<B, ALIGN, true>>::HEADER_SIZE
        };

        let b = <Bytes as Encoder<B, ALIGN, true>>::decode(&chunk, cur_offset)?;
        cur_offset += if <Bytes as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            ALIGN
        } else {
            <Bytes as Encoder<B, ALIGN, true>>::HEADER_SIZE
        };

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
    //
    //     // outer
    fn encode_header(
        &self,
        buf: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        let mut written = 0;

        if ctx.depth == 0 && <Self as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            // Top-level offset всегда 32 байта
            write_u32_aligned::<B, ALIGN>(buf, 32u32);
            ctx.hdr_ptr += 32;
            written += 32;
        }

        let initial_hdr_ptr = ctx.hdr_ptr;

        written += encode_field_header::<_, B, ALIGN>(&self.inner, buf, ctx)?;
        ctx.hdr_ptr = initial_hdr_ptr;

        written += encode_field_header::<_, B, ALIGN>(&self.balance, buf, ctx)?;
        ctx.hdr_ptr = ctx.hdr_size;

        Ok(written)
    }

    fn encode_tail(
        &self,
        buf: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        let mut written = 0;

        written += encode_field_tail::<_, B, ALIGN>(&self.inner, buf, ctx)?;
        written += encode_field_tail::<_, B, ALIGN>(&self.balance, buf, ctx)?;

        Ok(written)
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
        cur_offset += if <ExampleInner as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            ALIGN
        } else {
            <ExampleInner as Encoder<B, ALIGN, true>>::HEADER_SIZE
        };

        let balance = <U256 as Encoder<B, ALIGN, true>>::decode(&chunk, cur_offset)?;
        cur_offset += if <U256 as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            ALIGN
        } else {
            <ExampleInner as Encoder<B, ALIGN, true>>::HEADER_SIZE
        };

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

#[derive(Debug, Clone, PartialEq)]
struct ExampleOuter {
    example: Example,
    metadata: Bytes,
}

impl<B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, true> for ExampleOuter {
    const HEADER_SIZE: usize = {
        let mut size = 0;
        size += <Example as Encoder<B, ALIGN, true>>::HEADER_SIZE;
        size += <Bytes as Encoder<B, ALIGN, true>>::HEADER_SIZE;

        if <Self as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            size += align_up::<ALIGN>(32);
        }
        size
    };

    const IS_DYNAMIC: bool = {
        <Example as Encoder<B, ALIGN, true>>::IS_DYNAMIC
            || <Bytes as Encoder<B, ALIGN, true>>::IS_DYNAMIC
    };

    fn header_size(&self) -> usize {
        let mut size = 0;
        size += if <Example as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            ALIGN
        } else {
            <Example as Encoder<B, ALIGN, true>>::HEADER_SIZE
        };
        size += if <Bytes as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            ALIGN
        } else {
            <Bytes as Encoder<B, ALIGN, true>>::HEADER_SIZE
        };

        if <Self as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            size += ALIGN;
        }
        size
    }

    fn encode_header(
        &self,
        buf: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        let mut written = 0;

        if ctx.depth == 0 && <Self as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            write_u32_aligned::<B, ALIGN>(buf, 32u32);
            ctx.hdr_ptr += 32;
            written += 32;
            println!(
                "[ExampleOuter::encode_header()]: top level - writing offset: {:?}; ctx: {:?}",
                32, ctx
            );
        }

        // offset since the start of the container
        let initial_hdr_ptr = ctx.hdr_ptr;

        written += encode_field_header::<_, B, ALIGN>(&self.example, buf, ctx)?;
        println!("[ExampleOuter::encode_header()] ctx: {:?}", ctx);
        ctx.hdr_ptr = initial_hdr_ptr;

        written += encode_field_header::<_, B, ALIGN>(&self.metadata, buf, ctx)?;
        println!("[ExampleOuter::encode_header()] ctx: {:?}", ctx);
        ctx.hdr_ptr = ctx.hdr_size;

        Ok(written)
    }

    fn encode_tail(
        &self,
        buf: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        let mut written = 0;

        written += encode_field_tail::<_, B, ALIGN>(&self.example, buf, ctx)?;
        written += encode_field_tail::<_, B, ALIGN>(&self.metadata, buf, ctx)?;

        Ok(written)
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let (chunk, mut cur_offset) = if <Self as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            let tuple_relative_offset = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;
            (&buf.chunk()[offset + tuple_relative_offset..], 0)
        } else {
            (buf.chunk(), offset)
        };

        let example = <Example as Encoder<B, ALIGN, true>>::decode(&chunk, cur_offset)?;
        cur_offset += if <Example as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            ALIGN
        } else {
            <Example as Encoder<B, ALIGN, true>>::HEADER_SIZE
        };

        let metadata = <Bytes as Encoder<B, ALIGN, true>>::decode(&chunk, cur_offset)?;
        cur_offset += if <Bytes as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            ALIGN
        } else {
            <Bytes as Encoder<B, ALIGN, true>>::HEADER_SIZE
        };

        Ok(Self { example, metadata })
    }

    fn tail_size(&self, ctx: &mut EncodingContext) -> Result<usize, CodecError> {
        let mut size = 0;
        if <Example as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            size += <Example as Encoder<B, ALIGN, true>>::tail_size(&self.example, ctx)?;
        }
        if <Bytes as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            size += <Bytes as Encoder<B, ALIGN, true>>::tail_size(&self.metadata, ctx)?;
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

    #[ignore]
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

            struct ExampleOuterAlloy {
                ExampleAlloy example;
                bytes metadata;
            }
        }
        let inner = ExampleInnerAlloy {
            nums: vec![vec![1, 2, 3], vec![4, 5]],
            age: 42,
            tags: vec![7, 8, 9],
            b: Bytes::from("Hello World"),
        };

        let example = ExampleAlloy {
            inner,
            balance: U256::from(42),
        };

        let value = ExampleOuterAlloy {
            example,
            metadata: Bytes::from("Hello World!"),
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

    #[test]
    fn test_example_outer_solidity_encoding() {
        let inner = ExampleInner {
            nums: vec![vec![1, 2, 3], vec![4, 5]],
            age: 42,
            tags: vec![7, 8, 9],
            b: Bytes::from("Hello World"),
        };
        let example = Example {
            inner,
            balance: Uint::from(42),
        };
        let value = ExampleOuter {
            example,
            metadata: Bytes::from("Hello World!"),
        };
        assert_codec_sol(
            concat!(
                "0000000000000000000000000000000000000000000000000000000000000020", /* [0x0000] 0 = 32 */
                "0000000000000000000000000000000000000000000000000000000000000040", /* [0x0020] 32 = 64 */
                "0000000000000000000000000000000000000000000000000000000000000300", /* [0x0040] 64 = 768 */
            ),
            concat!(
                "0000000000000000000000000000000000000000000000000000000000000040", /* [0x0060] 96 = 64 */
                "000000000000000000000000000000000000000000000000000000000000002a", /* [0x0080] 128 = 42 */
                "0000000000000000000000000000000000000000000000000000000000000080", /* [0x00a0] 160 = 128 */
                "000000000000000000000000000000000000000000000000000000000000002a", /* [0x00c0] 192 = 42 */
                "00000000000000000000000000000000000000000000000000000000000001c0", /* [0x00e0] 224 = 448 */
                "0000000000000000000000000000000000000000000000000000000000000240", /* [0x0100] 256 = 576 */
                "0000000000000000000000000000000000000000000000000000000000000002", /* [0x0120] 288 = 2 */
                "0000000000000000000000000000000000000000000000000000000000000040", /* [0x0140] 320 = 64 */
                "00000000000000000000000000000000000000000000000000000000000000c0", /* [0x0160] 352 = 192 */
                "0000000000000000000000000000000000000000000000000000000000000003", /* [0x0180] 384 = 3 */
                "0000000000000000000000000000000000000000000000000000000000000001", /* [0x01a0] 416 = 1 */
                "0000000000000000000000000000000000000000000000000000000000000002", /* [0x01c0] 448 = 2 */
                "0000000000000000000000000000000000000000000000000000000000000003", /* [0x01e0] 480 = 3 */
                "0000000000000000000000000000000000000000000000000000000000000002", /* [0x0200] 512 = 2 */
                "0000000000000000000000000000000000000000000000000000000000000004", /* [0x0220] 544 = 4 */
                "0000000000000000000000000000000000000000000000000000000000000005", /* [0x0240] 576 = 5 */
                "0000000000000000000000000000000000000000000000000000000000000003", /* [0x0260] 608 = 3 */
                "0000000000000000000000000000000000000000000000000000000000000007", /* [0x0280] 640 = 7 */
                "0000000000000000000000000000000000000000000000000000000000000008", /* [0x02a0] 672 = 8 */
                "0000000000000000000000000000000000000000000000000000000000000009", /* [0x02c0] 704 = 9 */
                "000000000000000000000000000000000000000000000000000000000000000b", /* [0x02e0] 736 = 11 */
                "48656c6c6f20576f726c64000000000000000000000000000000000000000000", /* [0x0300] 768 = 0 */
                "000000000000000000000000000000000000000000000000000000000000000c", /* [0x0320] 800 = 12 */
                "48656c6c6f20576f726c64210000000000000000000000000000000000000000", /* [0x0340] 832 = 0 */
            ),
            &value,
        )
    }
}
