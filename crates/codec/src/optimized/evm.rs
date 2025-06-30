use crate::optimized::{
    ctx::EncodingContext,
    encoder::Encoder,
    error::CodecError,
    utils::{align_up, is_big_endian, read_bytes, read_u32_aligned, write_u32_aligned},
};
use alloy_primitives::{Address, Bytes, FixedBytes, Signed, Uint};
use byteorder::ByteOrder;
use bytes::{Buf, BufMut};

impl<B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool> Encoder<B, ALIGN, SOL_MODE> for Bytes {
    const HEADER_SIZE: usize = if SOL_MODE {
        align_up::<ALIGN>(32)
    } else {
        align_up::<ALIGN>(4) * 2
    };
    const IS_DYNAMIC: bool = true;

    #[allow(clippy::duplicate_code)]
    fn header_size(&self) -> usize {
        <Self as Encoder<B, ALIGN, SOL_MODE>>::HEADER_SIZE
    }

    fn encode_header(
        &self,
        out: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        if ctx.header_encoded {
            // Header already encoded; skip writing offset.
            return Ok(0);
        }
        let hdr_ptr_start = ctx.hdr_ptr;

        let offset = ctx.hdr_size + ctx.data_ptr - ctx.hdr_ptr ;

        if SOL_MODE {
            write_u32_aligned::<B, ALIGN>(out, offset);
            ctx.data_ptr += ALIGN as u32 + align_up::<ALIGN>(self.0.len()) as u32;
        } else {
            write_u32_aligned::<B, ALIGN>(out, offset);
            write_u32_aligned::<B, ALIGN>(out, self.0.len() as u32);
            ctx.data_ptr += self.0.len() as u32;
        }

        ctx.hdr_ptr += <Self as Encoder<B, ALIGN, SOL_MODE>>::HEADER_SIZE as u32;
        Ok((ctx.hdr_ptr - hdr_ptr_start) as usize)
    }

    fn encode_tail(
        &self,
        out: &mut impl BufMut,
        _ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        let start = out.remaining_mut();

        if SOL_MODE {
            // Write size before data
            write_u32_aligned::<B, ALIGN>(
                out,
                <Bytes as Encoder<B, ALIGN, SOL_MODE>>::len(self) as u32,
            );
        }
        out.put_slice(self);

        // Add padding to align the data section
        out.put_bytes(
            0,
            (ALIGN - (<Bytes as Encoder<B, ALIGN, SOL_MODE>>::len(self) % ALIGN)) % ALIGN,
        );

        Ok(start - out.remaining_mut())
    }

    /// Decodes `Bytes` from ABI-encoded buffer, supporting Solidity and Compact ABIs.
    ///
    /// # ABI encoding differences:
    ///
    /// - **Solidity ABI** (`SOL_MODE = true`): Encoded as `[offset → | length → data]`. `offset`
    ///   (data_offset) points directly to the **length** field. The actual data immediately follows
    ///   the length field. Therefore, we add `ALIGN` (32 bytes) to reach the data.
    ///
    /// - **Compact ABI** (`SOL_MODE = false`): Encoded as `[offset → length | data]`. `offset`
    ///   (data_offset) points directly to the actual **data**. The length field is stored
    ///   separately, immediately after the offset field itself. Thus, length is read directly after
    ///   the offset.
    ///
    /// This function transparently handles these differences.
    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let (data_offset, data_len) = if SOL_MODE {
            // Solidity ABI: offset → length → data
            let data_offset = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;
            let data_len = read_u32_aligned::<B, ALIGN>(buf, data_offset)? as usize;
            (data_offset + ALIGN, data_len)
        } else {
            // Compact ABI: offset → data, length stored next to offset field
            let data_offset = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;
            let data_len =
                read_u32_aligned::<B, ALIGN>(buf, offset + align_up::<ALIGN>(4))? as usize;
            // Offset is relative; add current offset for absolute positioning
            (data_offset + offset, data_len)
        };

        Ok(Bytes::from(read_bytes(buf, data_offset, data_len)?))
    }

    #[inline]
    fn len(&self) -> usize {
        self.0.len()
    }
}

impl<B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool> Encoder<B, ALIGN, SOL_MODE>
    for String
{
    const HEADER_SIZE: usize = if SOL_MODE {
        align_up::<ALIGN>(32)
    } else {
        align_up::<ALIGN>(4) * 2
    };
    const IS_DYNAMIC: bool = true;

    fn header_size(&self) -> usize {
        <Self as Encoder<B, ALIGN, SOL_MODE>>::HEADER_SIZE
    }

    fn encode_header(
        &self,
        out: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        let hdr_ptr_start = ctx.hdr_ptr;

        let offset = ctx.hdr_size - ctx.hdr_ptr + ctx.data_ptr;

        if SOL_MODE {
            write_u32_aligned::<B, ALIGN>(out, offset);
            ctx.data_ptr += ALIGN as u32 + align_up::<ALIGN>(self.len()) as u32;
        } else {
            write_u32_aligned::<B, ALIGN>(out, offset);
            write_u32_aligned::<B, ALIGN>(out, self.len() as u32);
            ctx.data_ptr += self.len() as u32;
        }

        ctx.hdr_ptr += <Self as Encoder<B, ALIGN, SOL_MODE>>::HEADER_SIZE as u32;
        Ok((ctx.hdr_ptr - hdr_ptr_start) as usize)
    }

    fn encode_tail(
        &self,
        out: &mut impl BufMut,
        _ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        let bytes = self.as_bytes();

        if SOL_MODE {
            // Write length prefix (aligned)
            write_u32_aligned::<B, ALIGN>(out, bytes.len() as u32);
        }

        out.put_slice(bytes);

        // Padding for alignment
        let padding = (ALIGN - (bytes.len() % ALIGN)) % ALIGN;
        if padding > 0 {
            out.put_bytes(0, padding);
        }

        Ok(bytes.len() + if SOL_MODE { ALIGN } else { 0 } + padding)
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let bytes = <Bytes as Encoder<B, ALIGN, SOL_MODE>>::decode(buf, offset)?;

        String::from_utf8(bytes.to_vec())
            .map_err(|_| CodecError::InvalidData("Invalid UTF-8 in string"))
    }

    #[inline]
    fn len(&self) -> usize {
        self.len()
    }
}

impl<const N: usize, B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool>
    Encoder<B, ALIGN, SOL_MODE> for FixedBytes<N>
{
    // Fixed-size byte array aligned appropriately
    const HEADER_SIZE: usize = if SOL_MODE { 32 } else { align_up::<ALIGN>(N) };
    const IS_DYNAMIC: bool = false;

    fn header_size(&self) -> usize {
        <FixedBytes<N> as Encoder<B, ALIGN, SOL_MODE>>::HEADER_SIZE
    }

    fn encode_header(
        &self,
        out: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        let initial_ptr = ctx.hdr_ptr;

        out.put_slice(&self.0);
        let padding = <FixedBytes<N> as Encoder<B, ALIGN, SOL_MODE>>::HEADER_SIZE - N;
        if padding > 0 {
            out.put_bytes(0, padding);
        }

        ctx.hdr_ptr += <FixedBytes<N> as Encoder<B, ALIGN, SOL_MODE>>::HEADER_SIZE as u32;

        Ok((ctx.hdr_ptr - initial_ptr) as usize)
    }

    fn encode_tail(
        &self,
        _: &mut impl BufMut,
        _: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        // FixedBytes are static, no tail encoding required
        Ok(0)
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let end_offset = offset + N;
        if buf.remaining() < end_offset {
            return Err(CodecError::BufferTooSmallMsg {
                expected: end_offset,
                actual: buf.remaining(),
                message: "Buffer too small for FixedBytes decoding",
            });
        }

        let mut data = [0u8; N];
        data.copy_from_slice(&buf.chunk()[offset..end_offset]);

        Ok(FixedBytes(data))
    }

    #[inline]
    fn len(&self) -> usize {
        N
    }
}

macro_rules! impl_evm_fixed {
    ($type:ty) => {
        impl<B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, false> for $type {
            const HEADER_SIZE: usize = <$type>::len_bytes();
            const IS_DYNAMIC: bool = false;

            /// Encode fixed-size bytes directly into the buffer.
            fn encode_header(
                &self,
                out: &mut impl BufMut,
                ctx: &mut EncodingContext,
            ) -> Result<usize, CodecError> {
                out.put_slice(self.as_slice());
                ctx.hdr_ptr += <Self as Encoder<B, ALIGN, false>>::HEADER_SIZE as u32;
                Ok(<Self as Encoder<B, ALIGN, false>>::HEADER_SIZE)
            }

            /// No separate tail for fixed bytes.
            fn encode_tail(
                &self,
                _: &mut impl BufMut,
                _: &mut EncodingContext,
            ) -> Result<usize, CodecError> {
                Ok(0)
            }

            /// Decode fixed-size bytes directly from the buffer.
            fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
                let size = <$type>::len_bytes();
                if buf.remaining() < offset + size {
                    return Err(CodecError::BufferTooSmallMsg {
                        expected: offset + size,
                        actual: buf.remaining(),
                        message: "Buffer too small to decode fixed bytes",
                    });
                }
                let data = buf.chunk()[offset..offset + size].to_vec();
                Ok(<$type>::from_slice(&data))
            }
        }

        // Solidity ABI mode: always padded to 32 bytes
        impl<B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, true> for $type {
            const HEADER_SIZE: usize = 32;
            const IS_DYNAMIC: bool = false;

            /// Encode fixed-size bytes padded to 32 bytes.
            fn encode_header(
                &self,
                out: &mut impl BufMut,
                ctx: &mut EncodingContext,
            ) -> Result<usize, CodecError> {
                out.put_bytes(0, 32 - Self::len_bytes());
                out.put_slice(self.as_slice());
                ctx.hdr_ptr += 32;
                Ok(32)
            }

            /// No separate tail for fixed bytes in Solidity ABI.
            fn encode_tail(
                &self,
                _: &mut impl BufMut,
                _: &mut EncodingContext,
            ) -> Result<usize, CodecError> {
                Ok(0)
            }

            /// Decode fixed-size bytes from 32-byte padded buffer.
            fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
                let size = <$type>::len_bytes();
                if buf.remaining() < offset + 32 {
                    return Err(CodecError::BufferTooSmallMsg {
                        expected: offset + 32,
                        actual: buf.remaining(),
                        message: "Buffer too small to decode fixed bytes",
                    });
                }
                let data = buf.chunk()[offset + 32 - size..offset + 32].to_vec();
                Ok(<$type>::from_slice(&data))
            }
        }
    };
}

impl_evm_fixed!(Address);

impl<
        const BITS: usize,
        const LIMBS: usize,
        B: ByteOrder,
        const ALIGN: usize,
        const SOL_MODE: bool,
    > Encoder<B, ALIGN, SOL_MODE> for Signed<BITS, LIMBS>
{
    const HEADER_SIZE: usize = align_up::<ALIGN>(Self::BYTES);
    const IS_DYNAMIC: bool = false;

    fn header_size(&self) -> usize {
        align_up::<ALIGN>(Self::BYTES)
    }

    fn encode_header(
        &self,
        out: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        // for Compact ABI, we need to align the header size - Signed can be larger than 4 bytes
        let total_size =
            align_up::<ALIGN>(<Self as Encoder<B, ALIGN, SOL_MODE>>::HEADER_SIZE.max(Self::BYTES));

        let pad_byte = if self.is_negative() { 0xFF } else { 0x00 };

        if crate::is_big_endian::<B>() {
            out.put_bytes(pad_byte, total_size - Self::BYTES);
            for limb in self.into_raw().as_limbs().iter().rev() {
                out.put_u64(*limb);
            }
        } else {
            for limb in self.into_raw().as_limbs().iter() {
                out.put_u64_le(*limb);
            }
            out.put_bytes(pad_byte, total_size - Self::BYTES);
        }
        ctx.hdr_ptr += total_size as u32;
        Ok(total_size)
    }

    fn encode_tail(
        &self,
        _out: &mut impl BufMut,
        _ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        // Static type has no tail
        Ok(0)
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        // for Compact ABI, we need to align the header size - Signed can be larger than 4 bytes
        let word_size =
            align_up::<ALIGN>(<Self as Encoder<B, ALIGN, SOL_MODE>>::HEADER_SIZE.max(Self::BYTES));

        if buf.remaining() < offset + word_size {
            return Err(CodecError::BufferTooSmallMsg {
                expected: offset + word_size,
                actual: buf.remaining(),
                message: "Buffer too small for decoding Signed type",
            });
        }
        // aligned slice with value
        let slice = &buf.chunk()[offset..offset + word_size];
        let value = if is_big_endian::<B>() {
            let v = &slice[word_size - Self::BYTES..];
            Self::from_raw(Uint::<BITS, LIMBS>::from_be_slice(v))
        } else {
            let v = &slice[..Self::BYTES];
            Self::from_raw(Uint::<BITS, LIMBS>::from_le_slice(v))
        };

        Ok(value)
    }
}

impl<
        const BITS: usize,
        const LIMBS: usize,
        B: ByteOrder,
        const ALIGN: usize,
        const SOL_MODE: bool,
    > Encoder<B, ALIGN, SOL_MODE> for Uint<BITS, LIMBS>
{
    const HEADER_SIZE: usize = align_up::<ALIGN>(Self::BYTES);
    const IS_DYNAMIC: bool = false;

    #[allow(clippy::duplicate_code)]
    fn header_size(&self) -> usize {
        align_up::<ALIGN>(Self::BYTES)
    }

    fn encode_header(
        &self,
        out: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        // Determine total aligned size for Uint encoding
        let total_size =
            align_up::<ALIGN>(<Self as Encoder<B, ALIGN, SOL_MODE>>::HEADER_SIZE.max(Self::BYTES));

        // Write padding and value according to endianness
        if crate::is_big_endian::<B>() {
            out.put_bytes(0x00, total_size - Self::BYTES);
            for limb in self.as_limbs().iter().rev() {
                out.put_u64(*limb);
            }
        } else {
            for limb in self.as_limbs().iter() {
                out.put_u64_le(*limb);
            }
            out.put_bytes(0x00, total_size - Self::BYTES);
        }

        ctx.hdr_ptr += total_size as u32;
        Ok(total_size)
    }

    fn encode_tail(
        &self,
        _out: &mut impl BufMut,
        _ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        // Static type has no tail
        Ok(0)
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        // Determine total aligned size for Uint decoding
        let word_size =
            align_up::<ALIGN>(<Self as Encoder<B, ALIGN, SOL_MODE>>::HEADER_SIZE.max(Self::BYTES));

        if buf.remaining() < offset + word_size {
            return Err(CodecError::BufferTooSmallMsg {
                expected: offset + word_size,
                actual: buf.remaining(),
                message: "Buffer too small for decoding Uint type",
            });
        }

        let slice = &buf.chunk()[offset..offset + word_size];
        let value = if is_big_endian::<B>() {
            // Big-endian: extract from the right side
            let v = &slice[word_size - Self::BYTES..];
            Uint::<BITS, LIMBS>::from_be_slice(v)
        } else {
            // Little-endian: extract from the left side
            let v = &slice[..Self::BYTES];
            Uint::<BITS, LIMBS>::from_le_slice(v)
        };

        Ok(value)
    }
}
#[cfg(test)]
mod tests {
    use crate::optimized::{
        ctx::EncodingContext,
        encoder::Encoder,
        utils::test_utils::{
            assert_codec_compact,
            assert_codec_sol,
            assert_roundtrip_compact,
            assert_roundtrip_sol,
        },
    };
    use alloy_primitives::{Address, Bytes, FixedBytes, I128, I256, U128, U256};
    use byteorder::{BigEndian, LittleEndian};
    use crate::optimized::utils::test_utils::assert_alloy_sol_roundtrip;

    mod bytes_compact {
        use super::*;
        use alloy_primitives::Bytes;

        #[test]
        fn test_empty_bytes() {
            let test_value = Bytes::new();
            let expected_header_hex = concat!(
                "08000000", // offset (8 bytes, immediately after header)
                "00000000"  // size = 0
            );
            let expected_tail_hex = ""; // no tail

            assert_codec_compact(expected_header_hex, expected_tail_hex, &test_value);
        }

        #[test]
        fn test_simple_bytes() {
            let test_value = Bytes::from_static(b"hello");
            let expected_header_hex = concat!(
                "08000000", // offset (8 bytes, immediately after header)
                "05000000", // size = 5
            );
            let expected_tail_hex = "68656c6c6f000000"; // "hello" + padding to align to 4 bytes

            assert_codec_compact(expected_header_hex, expected_tail_hex, &test_value);
        }

        #[test]
        fn test_large_bytes() {
            let data = (0..100).collect::<Vec<u8>>();
            let test_value = Bytes::copy_from_slice(&data);

            let expected_header_hex = concat!(
                "08000000", // offset (8 bytes, immediately after header)
                "64000000"  // length = 100
            );

            let expected_tail_hex = hex::encode(&data);

            assert_codec_compact(expected_header_hex, &expected_tail_hex, &test_value);
        }

        #[test]
        fn test_bytes_roundtrip() {
            let empty_bytes = Bytes::new();
            assert_roundtrip_compact(&empty_bytes);

            let hello_bytes = Bytes::from_static(b"hello world");
            assert_roundtrip_compact(&hello_bytes);

            let random_bytes: Bytes = (0..255).collect::<Vec<u8>>().into();
            assert_roundtrip_compact(&random_bytes);
        }
    }

    mod bytes_solidity {
        use super::*;
        use alloy_primitives::Bytes;

        #[test]
        fn test_empty_bytes_solidity() {
            let test_value = Bytes::new();
            let expected_header_hex =
                concat!("0000000000000000000000000000000000000000000000000000000000000020"); // offset (32 bytes)
            let expected_tail_hex = concat!(
                "0000000000000000000000000000000000000000000000000000000000000000" // length = 0
            );

            assert_codec_sol(expected_header_hex, expected_tail_hex, &test_value);
        }

        #[test]
        fn test_simple_bytes_solidity() {
            let test_value = Bytes::from_static(b"hello");
            let expected_header_hex =
                concat!("0000000000000000000000000000000000000000000000000000000000000020"); // offset (32 bytes)
            let expected_tail_hex = concat!(
                "0000000000000000000000000000000000000000000000000000000000000005", // length = 5
                "68656c6c6f000000000000000000000000000000000000000000000000000000" /* "hello" + padding */
            );

            assert_codec_sol(expected_header_hex, expected_tail_hex, &test_value);
        }

        #[test]
        fn test_large_bytes_solidity() {
            let data = (0..100).collect::<Vec<u8>>();
            let test_value = Bytes::copy_from_slice(&data);

            let expected_header_hex =
                concat!("0000000000000000000000000000000000000000000000000000000000000020"); // offset (32 bytes)

            let data_hex = hex::encode(&data);
            let padding = "00".repeat(28);

            let expected_tail_hex = format!(
                "{:064x}{}{}",
                100, // length = 100
                data_hex,
                padding
            );

            assert_codec_sol(&expected_header_hex, &expected_tail_hex, &test_value);
        }

        #[test]
        fn test_bytes_roundtrip_solidity() {
            let empty_bytes = Bytes::new();
            assert_roundtrip_sol(&empty_bytes);

            let hello_bytes = Bytes::from_static(b"hello world");
            assert_roundtrip_sol(&hello_bytes);

            let random_bytes: Bytes = (0..255).collect::<Vec<u8>>().into();
            assert_roundtrip_sol(&random_bytes);
        }
    }

    mod string_compact {
        use super::*;

        #[test]
        fn test_empty_string() {
            let test_value = String::new();
            let expected_header_hex = concat!(
                "08000000", // offset (8 bytes, immediately after header)
                "00000000"  // size = 0
            );
            let expected_tail_hex = ""; // no tail

            assert_codec_compact(expected_header_hex, expected_tail_hex, &test_value);
        }

        #[test]
        fn test_simple_string() {
            let test_value = String::from("hello");
            let expected_header_hex = concat!(
                "08000000", // offset (8 bytes, immediately after header)
                "05000000"  // size = 5
            );
            let expected_tail_hex = "68656c6c6f000000"; // "hello" + padding

            assert_codec_compact(expected_header_hex, expected_tail_hex, &test_value);
        }

        #[test]
        fn test_large_string() {
            let data = (0..100).map(|i| (i % 26 + 97) as u8).collect::<Vec<u8>>();
            let test_value = String::from_utf8(data.clone()).unwrap();

            let expected_header_hex = concat!(
                "08000000", // offset (8 bytes, immediately after header)
                "64000000"  // size = 100
            );

            let expected_tail_hex = hex::encode(&data);

            assert_codec_compact(expected_header_hex, &expected_tail_hex, &test_value);
        }

        #[test]
        fn test_string_roundtrip() {
            let empty_string = String::new();
            assert_roundtrip_compact(&empty_string);

            let hello_string = String::from("hello world");
            assert_roundtrip_compact(&hello_string);

            let random_string: String = (0u8..255).map(|i| (i % 26 + 97) as char).collect();
            assert_roundtrip_compact(&random_string);
        }
    }

    mod string_solidity {
        use super::*;

        #[test]
        fn test_empty_string_solidity() {
            let test_value = String::new();
            let expected_header_hex =
                concat!("0000000000000000000000000000000000000000000000000000000000000020"); // offset (32 bytes)
            let expected_tail_hex = concat!(
                "0000000000000000000000000000000000000000000000000000000000000000" // length = 0
            );

            assert_codec_sol(expected_header_hex, expected_tail_hex, &test_value);
        }

        #[test]
        fn test_simple_string_solidity() {
            let test_value = String::from("hello");
            let expected_header_hex =
                concat!("0000000000000000000000000000000000000000000000000000000000000020"); // offset (32 bytes)
            let expected_tail_hex = concat!(
                "0000000000000000000000000000000000000000000000000000000000000005", // length = 5
                "68656c6c6f000000000000000000000000000000000000000000000000000000" /* "hello" + padding */
            );

            assert_codec_sol(expected_header_hex, expected_tail_hex, &test_value);
        }

        #[test]
        fn test_large_string_solidity() {
            let data = (0..100).map(|i| (i % 26 + 97) as u8).collect::<Vec<u8>>();
            let test_value = String::from_utf8(data.clone()).unwrap();

            let expected_header_hex =
                concat!("0000000000000000000000000000000000000000000000000000000000000020"); // offset (32 bytes)

            let data_hex = hex::encode(&data);
            let padding = "00".repeat((32 - (data.len() % 32)) % 32);

            let expected_tail_hex = format!(
                "{:064x}{}{}",
                data.len(), // length = 100
                data_hex,
                padding
            );

            assert_codec_sol(&expected_header_hex, &expected_tail_hex, &test_value);
        }

        #[test]
        fn test_string_roundtrip_solidity() {
            let empty_string = String::new();
            assert_roundtrip_sol(&empty_string);

            let hello_string = String::from("hello world");
            assert_roundtrip_sol(&hello_string);

            let random_string: String = (0u8..255).map(|i| (i % 26 + 97) as char).collect();
            assert_roundtrip_sol(&random_string);
        }
    }

    mod signed_compact {
        use super::*;
        use alloy_primitives::{I128, I256};

        #[test]
        fn test_signed_compact() {
            assert_codec_compact(
                "2a000000000000000000000000000000",
                "",
                &I128::try_from(42).unwrap(),
            );
            assert_codec_compact(
                "d6ffffffffffffffffffffffffffffff",
                "",
                &I128::try_from(-42).unwrap(),
            );
            assert_codec_compact(
                "ffffffffffffffffffffffffffffffff",
                "",
                &I128::try_from(-1i32).unwrap(),
            );
            assert_codec_compact(
                "ffffffffffffffffffffffffffffff7f",
                "",
                &I128::try_from(I128::MAX).unwrap(),
            );
            assert_codec_compact(
                "00000000000000000000000000000080",
                "",
                &I128::try_from(I128::MIN).unwrap(),
            );
            assert_codec_compact(
                "2a00000000000000000000000000000000000000000000000000000000000000",
                "",
                &I256::try_from(42).unwrap(),
            );
            assert_codec_compact(
                "d6ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                "",
                &I256::try_from(-42).unwrap(),
            );
            assert_codec_compact(
                "0000000000000000000000000000000000000000000000000000000000000000",
                "",
                &I256::try_from(0).unwrap(),
            );
            assert_codec_compact(
                "d202964900000000000000000000000000000000000000000000000000000000",
                "",
                &I256::try_from(1234567890).unwrap(),
            );
            assert_codec_compact(
                "2efd69b6ffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                "",
                &I256::try_from(-1234567890).unwrap(),
            );
            assert_codec_compact(
                "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
                "",
                &I256::try_from(I256::MAX).unwrap(),
            );
            assert_codec_compact(
                "0000000000000000000000000000000000000000000000000000000000000080",
                "",
                &I256::try_from(I256::MIN).unwrap(),
            );
        }
    }

    mod signed_solidity {
        use super::*;
        use alloy_primitives::{I128, I256};

        #[test]
        fn test_signed_sol() {
            assert_codec_sol(
                "000000000000000000000000000000000000000000000000000000000000002a",
                "",
                &I128::try_from(42).unwrap(),
            );
            assert_codec_sol(
                "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffd6",
                "",
                &I128::try_from(-42).unwrap(),
            );
            assert_codec_sol(
                "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                "",
                &I128::try_from(-1i32).unwrap(),
            );
            assert_codec_sol(
                "000000000000000000000000000000007fffffffffffffffffffffffffffffff",
                "",
                &I128::try_from(I128::MAX).unwrap(),
            );
            assert_codec_sol(
                "ffffffffffffffffffffffffffffffff80000000000000000000000000000000",
                "",
                &I128::try_from(I128::MIN).unwrap(),
            );
            assert_codec_sol(
                "000000000000000000000000000000000000000000000000000000000000002a",
                "",
                &I256::try_from(42).unwrap(),
            );
            assert_codec_sol(
                "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffd6",
                "",
                &I256::try_from(-42).unwrap(),
            );
            assert_codec_sol(
                "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                "",
                &I256::try_from(-1i32).unwrap(),
            );
            assert_codec_sol(
                "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                "",
                &I256::try_from(I256::MAX).unwrap(),
            );
            assert_codec_sol(
                "8000000000000000000000000000000000000000000000000000000000000000",
                "",
                &I256::try_from(I256::MIN).unwrap(),
            );
        }
    }

    mod unsigned_compact {
        use super::*;
        use alloy_primitives::{U128, U256};

        #[test]
        fn test_uint_compact() {
            assert_codec_compact(
                "2a000000000000000000000000000000",
                "",
                &U128::try_from(42).unwrap(),
            );
            assert_codec_compact(
                "ffffffffffffffffffffffffffffffff",
                "",
                &U128::try_from(U128::MAX).unwrap(),
            );
            assert_codec_compact(
                "00000000000000000000000000000000",
                "",
                &U128::try_from(0u32).unwrap(),
            );
            assert_codec_compact(
                "2a00000000000000000000000000000000000000000000000000000000000000",
                "",
                &U256::try_from(42).unwrap(),
            );
            assert_codec_compact(
                "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                "",
                &U256::try_from(U256::MAX).unwrap(),
            );
            assert_codec_compact(
                "0000000000000000000000000000000000000000000000000000000000000000",
                "",
                &U256::try_from(0u32).unwrap(),
            );
        }
    }

    mod unsigned_solidity {
        use super::*;
        use alloy_primitives::{U128, U256};

        #[test]
        fn test_uint_sol() {
            assert_codec_sol(
                "000000000000000000000000000000000000000000000000000000000000002a",
                "",
                &U128::try_from(42).unwrap(),
            );
            assert_codec_sol(
                "00000000000000000000000000000000ffffffffffffffffffffffffffffffff",
                "",
                &U128::try_from(U128::MAX).unwrap(),
            );
            assert_codec_sol(
                "0000000000000000000000000000000000000000000000000000000000000000",
                "",
                &U128::try_from(0u32).unwrap(),
            );
            assert_codec_sol(
                "000000000000000000000000000000000000000000000000000000000000002a",
                "",
                &U256::try_from(42).unwrap(),
            );
            assert_codec_sol(
                "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                "",
                &U256::try_from(U256::MAX).unwrap(),
            );
            assert_codec_sol(
                "0000000000000000000000000000000000000000000000000000000000000000",
                "",
                &U256::try_from(0u32).unwrap(),
            );
        }
    }

    mod fixed_bytes_compact {
        use super::*;

        #[test]
        fn test_fixed_bytes_encode_decode() {
            let test_value = FixedBytes::<8>::from_slice(b"ABCDEFGH");

            let expected_header_hex = concat!(
                "41424344", // ABCD
                "45464748"  // EFGH
            );

            assert_codec_compact(expected_header_hex, "", &test_value);
        }

        #[test]
        fn test_fixed_bytes_overflow() {
            let buf = [0u8; 4]; // buffer too small for FixedBytes<8>
            let decode_result =
                <FixedBytes<8> as Encoder<LittleEndian, 4, false>>::decode(&buf.as_ref(), 0);
            assert!(
                decode_result.is_err(),
                "Expected overflow error due to small buffer"
            );
        }
    }

    mod address_compact {
        use super::*;

        #[test]
        fn test_address_codec() {
            let addr = Address::from_slice(&[0x11; 20]);

            let expected_header_hex = "1111111111111111111111111111111111111111";
            let expected_tail_hex = ""; // no tail for fixed bytes

            assert_codec_compact(expected_header_hex, expected_tail_hex, &addr);
        }

        #[test]
        fn test_address_decode_overflow() {
            let buf = vec![0x11; 15]; // shorter than required (20 bytes)
            let result = <Address as Encoder<BigEndian, 4, false>>::decode(&buf.as_slice(), 0);
            assert!(
                result.is_err(),
                "Expected decoding error due to buffer overflow"
            );
        }
    }

    mod address_solidity {
        use super::*;

        #[test]
        fn test_address_codec() {
            let addr = Address::from_slice(&[0x11; 20]);

            let expected_header_hex =
                "0000000000000000000000001111111111111111111111111111111111111111";
            let expected_tail_hex = ""; // no tail for fixed bytes

            assert_codec_sol(expected_header_hex, expected_tail_hex, &addr);
        }
    }

    #[test]
    fn test_alloy_sol_compatibility() {
        use alloy_sol_types::SolValue;
        use crate::optimized::encoder::Encoder;


        // Test Bytes - edge cases
        assert_alloy_sol_roundtrip(&Bytes::new(), "Bytes::empty");
        assert_alloy_sol_roundtrip(&Bytes::from(vec![0x00]), "Bytes::single_zero");
        assert_alloy_sol_roundtrip(&Bytes::from(vec![0xff]), "Bytes::single_ff");
        assert_alloy_sol_roundtrip(&Bytes::from(vec![0x01, 0x02, 0x03, 0x04, 0x05]), "Bytes::5_bytes");
        assert_alloy_sol_roundtrip(&Bytes::from(vec![0xaa; 31]), "Bytes::31_bytes");
        assert_alloy_sol_roundtrip(&Bytes::from(vec![0xbb; 32]), "Bytes::32_bytes");
        assert_alloy_sol_roundtrip(&Bytes::from(vec![0xcc; 33]), "Bytes::33_bytes");
        assert_alloy_sol_roundtrip(&Bytes::from(vec![0xdd; 64]), "Bytes::64_bytes");
        assert_alloy_sol_roundtrip(&Bytes::from(vec![0xee; 100]), "Bytes::100_bytes");

        // Test String - edge cases
        assert_alloy_sol_roundtrip(&String::new(), "String::empty");
        assert_alloy_sol_roundtrip(&String::from("a"), "String::single_char");
        assert_alloy_sol_roundtrip(&String::from("hello"), "String::hello");
        assert_alloy_sol_roundtrip(&String::from("0123456789abcdef0123456789abcde"), "String::31_chars");
        assert_alloy_sol_roundtrip(&String::from("0123456789abcdef0123456789abcdef"), "String::32_chars");
        assert_alloy_sol_roundtrip(&String::from("0123456789abcdef0123456789abcdef0"), "String::33_chars");
        assert_alloy_sol_roundtrip(&String::from("🦀"), "String::unicode_emoji");
        assert_alloy_sol_roundtrip(&String::from("Hello, 世界! 🌍"), "String::mixed_unicode");
        assert_alloy_sol_roundtrip(&"a".repeat(100), "String::100_chars");

        // Test FixedBytes - various sizes
        assert_alloy_sol_roundtrip(&FixedBytes::<1>::from([0x42]), "FixedBytes<1>");
        assert_alloy_sol_roundtrip(&FixedBytes::<4>::from([0x01, 0x02, 0x03, 0x04]), "FixedBytes<4>");
        assert_alloy_sol_roundtrip(&FixedBytes::<8>::from([0xff; 8]), "FixedBytes<8>");
        assert_alloy_sol_roundtrip(&FixedBytes::<16>::from([0xaa; 16]), "FixedBytes<16>");
        assert_alloy_sol_roundtrip(&FixedBytes::<20>::from([0xbb; 20]), "FixedBytes<20>");
        assert_alloy_sol_roundtrip(&FixedBytes::<32>::from([0xcc; 32]), "FixedBytes<32>");

        // Test Address
        assert_alloy_sol_roundtrip(&Address::ZERO, "Address::ZERO");
        assert_alloy_sol_roundtrip(&Address::from([0xff; 20]), "Address::MAX");
        assert_alloy_sol_roundtrip(&Address::from([
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
            0x01, 0x23, 0x45, 0x67
        ]), "Address::pattern");

        // Test U256 - edge cases
        assert_alloy_sol_roundtrip(&U256::ZERO, "U256::ZERO");
        assert_alloy_sol_roundtrip(&U256::from(1u64), "U256::ONE");
        assert_alloy_sol_roundtrip(&U256::from(42u64), "U256::42");
        assert_alloy_sol_roundtrip(&U256::from(u64::MAX), "U256::u64_max");
        assert_alloy_sol_roundtrip(&U256::from(u128::MAX), "U256::u128_max");
        assert_alloy_sol_roundtrip(&U256::MAX, "U256::MAX");
        assert_alloy_sol_roundtrip(&U256::from_be_bytes([
            0x80, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0
        ]), "U256::high_bit");
    }
}
