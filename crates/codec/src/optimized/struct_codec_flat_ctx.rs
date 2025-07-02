#![allow(warnings)]
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
use core::fmt::{Debug, Pointer};

// Encoding pointers calculation for container:
//
// hdr_ptr — start position of the current container's header (0 for top-level).
// header_size = number_of_fields × field_header_size.
// field_header_size = is_dynamic(field) ? 32 : field::HEADER_SIZE; (static fields encoded inplace)
//
// ```
// data_ptr = hdr_ptr + header_size.
// ```
//
// For top-level containers, hdr_ptr = 0, thus data_ptr = header_size, typically 32 bytes. Even if
// we encode struct - it's actually tuple under the hood. So you can think about it as single top
// level container. But the rule is the same as for other container. For nested containers, hdr_ptr
// points into the parent's data section; thus data_ptr is computed accordingly.
//
// Offsets for dynamic fields are always calculated relative to the current container's header
// start: ```
// offset = data_ptr - hdr_ptr
// ```

/// Encodes the header portion for a field.
///
/// - For dynamic fields: only writes the offset to the header, pointing to the field's data.
/// - For static fields: directly encodes the field's value into the header.
pub fn encode_field_header<T, B, const ALIGN: usize>(
    field: &T,
    buf: &mut impl BufMut,
    ctx: &mut EncodingContext,
) -> Result<usize, CodecError>
where
    T: Encoder<B, ALIGN, true> + Debug,
    B: ByteOrder,
{
    if T::IS_DYNAMIC {
        println!("[assert_codec_sol] // encode field header start...");
        println!("[assert_codec_sol] // ctx.hdr_ptr: {:?}", ctx.hdr_ptr);
        println!("[assert_codec_sol] // ctx.data_ptr: {:?}", ctx.data_ptr);
        // Calculate offset from header position to the start of data
        let offset = ctx.data_ptr - ctx.hdr_ptr;

        // Write offset to the buffer
        write_u32_aligned::<B, ALIGN>(buf, offset);
        ctx.hdr_ptr += 32;
        // >ERROR was here - we don't need to advance hdr ptr till to the end of container
        // Advance header pointer by offset length (1 word)
        // ctx.hdr_ptr += ALIGN as u32;

        // Reserve space in data pointer for the tail portion
        // если здесь отнять 32 то первый тест проходит!!!
        // tail size включает в себя вложенные хедеры!!! он не включает только
        let tail_size = T::tail_size(field, &mut EncodingContext::new())?;
        let elements_offsets = T::header_size(field); // header size
        println!("[assert_codec_sol] // T::tail_size: {}", tail_size);
        println!(
            "[assert_codec_sol] // T::elements_offsets: {:?}",
            elements_offsets
        );

        ctx.data_ptr += tail_size as u32 + elements_offsets as u32;
        println!("[assert_codec_sol] // ctx.hdr_ptr: {:?}", ctx.hdr_ptr);
        println!("[assert_codec_sol] // ctx.data_ptr: {:?}", ctx.data_ptr);
        println!("[assert_codec_sol] // end");
        println!("...");
        println!("...");
        Ok(ALIGN)
    } else {
        // For static fields, encode the value directly into the header
        let written = T::encode_header(field, buf, ctx)?;
        ctx.hdr_ptr += written as u32; // advance header pointer by written bytes
        Ok(written)
    }
}

// "0000000000000000000000000000000000000000000000000000000000000020", // [0x0000] 0 = 32
// "000000000000000000000000000000000000000000000000000000000000002a", // [0x0020] 32 = 42
// "0000000000000000000000000000000000000000000000000000000000000180", // [0x0040] 64 = 384
// "0000000000000000000000000000000000000000000000000000000000000220", // [0x0060] 96 = 544

/// Encodes the tail portion (actual data) of a dynamic field.
///
/// - For dynamic fields: encodes the field's header (excluding offset, which is already written)
///   and data.
/// - For static fields: no encoding is done here, since static fields have no separate tail.
pub fn encode_field_tail<T, B, const ALIGN: usize>(
    field: &T,
    buf: &mut impl BufMut,
    ctx: &mut EncodingContext,
) -> Result<usize, CodecError>
where
    T: Encoder<B, ALIGN, true> + Debug,
    B: ByteOrder,
{
    let mut written = 0;
    println!("[encode_field_tail] field: {:?}", field);
    println!(
        "[encode_field_tail] field.header_size(): {:?} field.len(): {:?}",
        field.header_size(),
        field.len()
    );
    println!("[encode_field_tail] current ctx: {:#?}", ctx);

    if T::IS_DYNAMIC {
        let header_size = T::header_size(&field) as u32;

        // Create a nested context; offset is already written at a higher level.
        let mut nested_ctx = ctx.nested_struct(header_size, true);
        println!("[encode_field_tail] nested_ctx123: {:#?}", nested_ctx);

        // Encode field header (excluding offset) and the actual data
        written += field.encode_header(buf, &mut nested_ctx)?;
        println!(
            "[encode_field_tail] nested ctx after encode header: {:?}",
            nested_ctx
        );

        written += field.encode_tail(buf, &mut nested_ctx)?;

        println!(
            "[encode_field_tail] nested ctx after encode tail: {:?}",
            nested_ctx
        );

        // Update parent context's data pointer after nested encoding
        ctx.data_ptr += written as u32;
        // ctx.data_ptr = nested_ctx.data_ptr;

        println!("written: {}", written);

        Ok(written)
    } else {
        // Static fields don't encode tail data separately.
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
    // header size without container offset. Only offsets and static data

    const HEADER_SIZE: usize = {
        let mut size = 0;

        if <Self as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            size += 32
        } else {
            size += <Vec<Vec<u32>> as Encoder<B, ALIGN, true>>::HEADER_SIZE;
            size += <u32 as Encoder<B, ALIGN, true>>::HEADER_SIZE;
            size += <Vec<u32> as Encoder<B, ALIGN, true>>::HEADER_SIZE;
            size += <Bytes as Encoder<B, ALIGN, true>>::HEADER_SIZE;
        }
        size
    };

    const IS_DYNAMIC: bool = {
        <Vec<Vec<u32>> as Encoder<B, ALIGN, true>>::IS_DYNAMIC
            || <u32 as Encoder<B, ALIGN, true>>::IS_DYNAMIC
            || <Vec<u8> as Encoder<B, ALIGN, true>>::IS_DYNAMIC
            || <Bytes as Encoder<B, ALIGN, true>>::IS_DYNAMIC
    };

    // Elements header sizes
    fn header_size(&self) -> usize {
        let mut size = 0;
        size += <Vec<Vec<u32>> as Encoder<B, ALIGN, true>>::HEADER_SIZE;
        size += <u32 as Encoder<B, ALIGN, true>>::HEADER_SIZE;
        size += <Vec<u32> as Encoder<B, ALIGN, true>>::HEADER_SIZE;
        size += <Bytes as Encoder<B, ALIGN, true>>::HEADER_SIZE;

        size
    }

    fn encode_header(
        &self,
        buf: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        let mut written = 0 as u32;

        if ctx.depth == 0 && <Self as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            // Top-level container offset always 32
            write_u32_aligned::<B, ALIGN>(buf, 32u32);
            println!("[encode_header] offset: {:?}", 32);
            ctx.hdr_ptr += 32;
            written += 32;
        }

        // Save initial header pointer to calculate offsets consistently

        // Encode each field's header sequentially (hdr_ptr stays fixed within container)
        written += encode_field_header::<_, B, ALIGN>(&self.nums, buf, ctx)? as u32;
        written += encode_field_header::<_, B, ALIGN>(&self.age, buf, ctx)? as u32;
        written += encode_field_header::<_, B, ALIGN>(&self.tags, buf, ctx)? as u32;
        written += encode_field_header::<_, B, ALIGN>(&self.b, buf, ctx)? as u32;

        Ok(written as usize)
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
//
impl<B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, true> for Example {
    const HEADER_SIZE: usize = {
        let mut size = 0;
        if <Self as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            size = align_up::<ALIGN>(32);
        } else {
            size += <ExampleInner as Encoder<B, ALIGN, true>>::HEADER_SIZE;
            size += <U256 as Encoder<B, ALIGN, true>>::HEADER_SIZE;
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

        written += encode_field_header::<_, B, ALIGN>(&self.inner, buf, ctx)?;
        written += encode_field_header::<_, B, ALIGN>(&self.balance, buf, ctx)?;

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
            size += <ExampleInner as Encoder<B, ALIGN, true>>::header_size(&self.inner);
            size += <ExampleInner as Encoder<B, ALIGN, true>>::tail_size(&self.inner, ctx)?;
        }
        if <U256 as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            size += <U256 as Encoder<B, ALIGN, true>>::tail_size(&self.balance, ctx)?;
        }

        size += <ExampleInner as Encoder<B, ALIGN, true>>::HEADER_SIZE;

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

        if <Self as Encoder<B, ALIGN, true>>::IS_DYNAMIC {
            size += align_up::<ALIGN>(32);
        } else {
            size += <Example as Encoder<B, ALIGN, true>>::HEADER_SIZE;
            size += <Bytes as Encoder<B, ALIGN, true>>::HEADER_SIZE;
        }
        size
    };

    const IS_DYNAMIC: bool = {
        <Example as Encoder<B, ALIGN, true>>::IS_DYNAMIC
            || <Bytes as Encoder<B, ALIGN, true>>::IS_DYNAMIC
    };

    // only elements sizes or offsets
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

        written += encode_field_header::<_, B, ALIGN>(&self.example, buf, ctx)?;
        written += encode_field_header::<_, B, ALIGN>(&self.metadata, buf, ctx)?;

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
    use crate::optimized::utils::test_utils::{assert_codec_sol_with_ctx, print_encoded};
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

        // full header size = top level offset + actual elements header sizes
        let full_header_size = (<ExampleInner as Encoder<BigEndian, 32, true>>::HEADER_SIZE
            + <ExampleInner as Encoder<BigEndian, 32, true>>::header_size(&value))
            as u32;
        let mut ctx = EncodingContext::with_hs(full_header_size);

        assert_codec_sol_with_ctx(
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
            &mut ctx,
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
        // full header size = top level offset + actual elements header sizes
        let full_hs = <Example as Encoder<BigEndian, 32, true>>::header_size(&value) as u32
            + <Example as Encoder<BigEndian, 32, true>>::HEADER_SIZE as u32;

        assert_eq!(full_hs, 96);

        let mut ctx = EncodingContext::with_hs(full_hs);

        assert_codec_sol_with_ctx(
            concat!(
                "0000000000000000000000000000000000000000000000000000000000000020", /* [0x0000] 0 = 32 */
                "0000000000000000000000000000000000000000000000000000000000000040", /* [0x0020] 32 = 64 */
                "000000000000000000000000000000000000000000000000000000000000002a", /* [0x0040] 64 = 42 */
            ),
            concat!(
                // ExampleInner
                // Header
                // offset to nums [[0x00e0] 224 ] 96 + 128 = 224
                "0000000000000000000000000000000000000000000000000000000000000080", /* [0x0060] 96 = 128 */
                // static field age
                "000000000000000000000000000000000000000000000000000000000000002a", /* [0x0080] 128 = 42 */
                // offset to tags [[0x0220] 544] 96 + 448 =  544
                "00000000000000000000000000000000000000000000000000000000000001c0", /* [0x00a0] 160 = 448 */
                // offset to b [[0x02a0] 672]  96 + 576 = 672
                "0000000000000000000000000000000000000000000000000000000000000240", /* [0x00c0] 192 = 576 */
                // Inner data
                // nums len
                "0000000000000000000000000000000000000000000000000000000000000002", /* [0x00e0] 224 = 2 */
                // offset to nums[0] target idx [[0x0140] 320] 256 + 64 = 320
                "0000000000000000000000000000000000000000000000000000000000000040", /* [0x0100] 256 = 64 */
                // offset to nums[1] target idx [[0x01c0] 448] 256 + 192 = 448
                "00000000000000000000000000000000000000000000000000000000000000c0", /* [0x0120] 288 = 192 */
                // nums[0]
                "0000000000000000000000000000000000000000000000000000000000000003", /* [0x0140] 320 = 3 */
                "0000000000000000000000000000000000000000000000000000000000000001", /* [0x0160] 352 = 1 */
                "0000000000000000000000000000000000000000000000000000000000000002", /* [0x0180] 384 = 2 */
                "0000000000000000000000000000000000000000000000000000000000000003", /* [0x01a0] 416 = 3 */
                // nums[1]
                "0000000000000000000000000000000000000000000000000000000000000002", /* [0x01c0] 448 = 2 */
                "0000000000000000000000000000000000000000000000000000000000000004", /* [0x01e0] 480 = 4 */
                "0000000000000000000000000000000000000000000000000000000000000005", /* [0x0200] 512 = 5 */
                // tags
                "0000000000000000000000000000000000000000000000000000000000000003", /* [0x0220] 544 = 3 */
                "0000000000000000000000000000000000000000000000000000000000000007", /* [0x0240] 576 = 7 */
                "0000000000000000000000000000000000000000000000000000000000000008", /* [0x0260] 608 = 8 */
                "0000000000000000000000000000000000000000000000000000000000000009", /* [0x0280] 640 = 9 */
                // b
                "000000000000000000000000000000000000000000000000000000000000000b", /* [0x02a0] 672 = 11 */
                "48656c6c6f20576f726c64000000000000000000000000000000000000000000", /* [0x02c0] 704 = 0 */
            ),
            &value,
            &mut ctx,
        );
    }
    #[ignore]
    #[test]
    fn test_example_solidity_encoding_tails_headers() {
        let inner = ExampleInner {
            nums: vec![vec![1, 2, 3], vec![4, 5]],
            age: 42,
            tags: vec![7, 8, 9],
            b: Bytes::from("Hello World"),
        };

        {
            let value = inner.nums.clone();
            let mut ctx = EncodingContext::with_hs(32);

            // full size without 32 bytes offset
            let tail_size =
                <Vec<Vec<u32>> as Encoder<BigEndian, 32, true>>::tail_size(&value, &mut ctx)
                    .unwrap();
            assert_eq!(tail_size, 320);

            let value = 42;
            let mut ctx = EncodingContext::with_hs(0);
            let tail_size =
                <u32 as Encoder<BigEndian, 32, true>>::tail_size(&value, &mut ctx).unwrap();
            assert_eq!(tail_size, 0, "static types tail len should be 0");

            let value = inner.tags.clone();
            let mut ctx = EncodingContext::with_hs(32);
            let tail_size =
                <Vec<u32> as Encoder<BigEndian, 32, true>>::tail_size(&value, &mut ctx).unwrap();
            assert_eq!(tail_size, 128,);

            let value = inner.b.clone();
            let mut ctx = EncodingContext::with_hs(0);
            let tail_size =
                <Bytes as Encoder<BigEndian, 32, true>>::tail_size(&value, &mut ctx).unwrap();
            assert_eq!(tail_size, 64);

            // 320+128+64 = 512
        }
        let mut ctx = EncodingContext::with_hs(32);
        let inner_tail_size =
            <ExampleInner as Encoder<BigEndian, 32, true>>::tail_size(&inner.clone(), &mut ctx)
                .unwrap();
        println!("inner tail size: {}", inner_tail_size);
        assert_eq!(inner_tail_size, 512);

        let inner_header_size =
            <ExampleInner as Encoder<BigEndian, 32, true>>::header_size(&inner.clone());
        assert_eq!(inner_header_size, 128);

        let value = Example {
            inner,
            balance: Uint::from(42),
        };

        let mut ctx = EncodingContext::with_hs(32);
        let value_tail_size =
            <Example as Encoder<BigEndian, 32, true>>::tail_size(&value, &mut ctx).unwrap();
        assert_eq!(value_tail_size, 512);

        let value_header_size = <Example as Encoder<BigEndian, 32, true>>::header_size(&value);
        assert_eq!(value_header_size, 64);
        //
        // // full header size = top level offset + actual elements header sizes
        // let full_hs = <Example as Encoder<BigEndian, 32, true>>::header_size(&value) as u32
        //     + <Example as Encoder<BigEndian, 32, true>>::HEADER_SIZE as u32;
        //
        // assert_eq!(full_hs, 96);
        //
        // let mut ctx = EncodingContext::with_hs(full_hs);
        //
        // assert_codec_sol_with_ctx(
        //     concat!(
        //         "0000000000000000000000000000000000000000000000000000000000000020", /* [0x0000] 0
        // = 32 */
        //         "0000000000000000000000000000000000000000000000000000000000000040", /* [0x0020]
        // 32 = 64 */
        //         "000000000000000000000000000000000000000000000000000000000000002a", /* [0x0040]
        // 64 = 42 */     ),
        //     concat!(
        //         // ExampleInner
        //         // Header
        //         // offset to nums [[0x00e0] 224 ] 96 + 128 = 224
        //         "0000000000000000000000000000000000000000000000000000000000000080", /* [0x0060]
        // 96 = 128 */         // static field age
        //         "000000000000000000000000000000000000000000000000000000000000002a", /* [0x0080]
        // 128 = 42 */         // offset to tags [[0x0220] 544] 96 + 448 =  544
        //         "00000000000000000000000000000000000000000000000000000000000001c0", /* [0x00a0]
        // 160 = 448 */         // offset to b [[0x02a0] 672]  96 + 576 = 672
        //         "0000000000000000000000000000000000000000000000000000000000000240", /* [0x00c0]
        // 192 = 576 */         // Inner data
        //         // nums len
        //         "0000000000000000000000000000000000000000000000000000000000000002", /* [0x00e0]
        // 224 = 2 */         // offset to nums[0] target idx [[0x0140] 320] 256 + 64 = 320
        //         "0000000000000000000000000000000000000000000000000000000000000040", /* [0x0100]
        // 256 = 64 */         // offset to nums[1] target idx [[0x01c0] 448] 256 + 192 =
        // 448         "00000000000000000000000000000000000000000000000000000000000000c0",
        // /* [0x0120] 288 = 192 */         // nums[0]
        //         "0000000000000000000000000000000000000000000000000000000000000003", /* [0x0140]
        // 320 = 3 */
        //         "0000000000000000000000000000000000000000000000000000000000000001", /* [0x0160]
        // 352 = 1 */
        //         "0000000000000000000000000000000000000000000000000000000000000002", /* [0x0180]
        // 384 = 2 */
        //         "0000000000000000000000000000000000000000000000000000000000000003", /* [0x01a0]
        // 416 = 3 */         // nums[1]
        //         "0000000000000000000000000000000000000000000000000000000000000002", /* [0x01c0]
        // 448 = 2 */
        //         "0000000000000000000000000000000000000000000000000000000000000004", /* [0x01e0]
        // 480 = 4 */
        //         "0000000000000000000000000000000000000000000000000000000000000005", /* [0x0200]
        // 512 = 5 */         // tags
        //         "0000000000000000000000000000000000000000000000000000000000000003", /* [0x0220]
        // 544 = 3 */
        //         "0000000000000000000000000000000000000000000000000000000000000007", /* [0x0240]
        // 576 = 7 */
        //         "0000000000000000000000000000000000000000000000000000000000000008", /* [0x0260]
        // 608 = 8 */
        //         "0000000000000000000000000000000000000000000000000000000000000009", /* [0x0280]
        // 640 = 9 */         // b
        //         "000000000000000000000000000000000000000000000000000000000000000b", /* [0x02a0]
        // 672 = 11 */
        //         "48656c6c6f20576f726c64000000000000000000000000000000000000000000", /* [0x02c0]
        // 704 = 0 */     ),
        //     &value,
        //     &mut ctx,
        // );
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

        // full header size = top level offset + actual elements header sizes
        let full_hs = <ExampleOuter as Encoder<BigEndian, 32, true>>::header_size(&value) as u32
            + <ExampleOuter as Encoder<BigEndian, 32, true>>::HEADER_SIZE as u32;

        assert_eq!(full_hs, 96);

        let mut ctx = EncodingContext::with_hs(full_hs);

        assert_codec_sol_with_ctx(
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
            &mut ctx,
        )
    }
}
