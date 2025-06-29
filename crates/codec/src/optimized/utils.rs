use crate::optimized::error::CodecError;
use byteorder::ByteOrder;
use bytes::{Buf, BufMut, Bytes};

/// Returns `true` if the byte order is big-endian.
pub fn is_big_endian<B: ByteOrder>() -> bool {
    B::read_u16(&[0x12, 0x34]) == 0x1234
}

/// Rounds up `offset` to the nearest multiple of `ALIGN`.
/// `ALIGN` must be a power of two.
#[inline]
pub const fn align_up<const ALIGN: usize>(offset: usize) -> usize {
    (offset + ALIGN - 1) & !(ALIGN - 1)
}

/// Writes a u32 to the buffer with required alignment and endianness.
/// For big-endian: pads first, then writes the value.
/// For little-endian: writes the value, then pads.
pub fn write_u32_aligned<B: ByteOrder, const ALIGN: usize>(buf: &mut impl BufMut, value: u32) {
    let aligned_value_size = align_up::<ALIGN>(4);

    if is_big_endian::<B>() {
        let padding = aligned_value_size - 4;
        if padding > 0 {
            buf.put_bytes(0, padding);
        }
        buf.put_u32(value);
    } else {
        buf.put_u32_le(value);
        let padding = aligned_value_size - 4;
        if padding > 0 {
            buf.put_bytes(0, padding);
        }
    }
}

/// Reads an aligned u32 from a buffer at the given offset.
/// Returns an error if the buffer does not have enough bytes.
pub fn read_u32_aligned<B: ByteOrder, const ALIGN: usize>(
    buf: &impl Buf,
    offset: usize,
) -> Result<u32, CodecError> {
    let word_size = align_up::<ALIGN>(4);
    let end = offset + word_size;

    if buf.remaining() < end {
        return Err(CodecError::BufferTooSmallMsg {
            expected: end,
            actual: buf.remaining(),
            message: "read_u32_aligned",
        });
    }
    if is_big_endian::<B>() {
        Ok(B::read_u32(&buf.chunk()[end - 4..end]))
    } else {
        Ok(B::read_u32(&buf.chunk()[offset..offset + 4]))
    }
}

/// Reads raw bytes from a buffer given explicit offset and length.
/// Creates a `Bytes` object from the slice (with allocation).
pub fn read_bytes(buf: &impl Buf, offset: usize, len: usize) -> Result<Bytes, CodecError> {
    if buf.remaining() < offset + len {
        return Err(CodecError::BufferTooSmallMsg {
            expected: offset + len,
            actual: buf.remaining(),
            message: "Buffer too small to read requested bytes",
        });
    }

    Ok(Bytes::copy_from_slice(&buf.chunk()[offset..offset + len]))
}

/// Reads header information and returns (data_offset, data_length).
/// Solidity and WASM modes are handled explicitly.
pub fn read_bytes_header<B: ByteOrder, const ALIGN: usize>(
    buf: &impl Buf,
    offset: usize,
    sol_mode: bool,
) -> Result<(usize, usize), CodecError> {
    if sol_mode {
        read_bytes_header_solidity::<B, ALIGN>(buf, offset)
    } else {
        read_bytes_header_compact::<B, ALIGN>(buf, offset)
    }
}

/// Reads WASM-compatible header:
/// Returns `(data_offset, data_length)` directly from two consecutive u32 fields.
pub fn read_bytes_header_compact<B: ByteOrder, const ALIGN: usize>(
    buf: &impl Buf,
    offset: usize,
) -> Result<(usize, usize), CodecError> {
    println!("read_bytes_header_compact");

    let word = align_up::<ALIGN>(4);
    let required_size = offset + word * 2;

    if required_size > buf.remaining() {
        return Err(CodecError::BufferTooSmallMsg {
            expected: required_size,
            actual: buf.remaining(),
            message: "Compact header: insufficient buffer size",
        });
    }

    let data_offset = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;
    let data_len = read_u32_aligned::<B, ALIGN>(buf, offset + word)? as usize;

    // we need to add offset - since data_offset is relative to the start of the buffer (offset)
    Ok((data_offset + offset, data_len))
}

pub fn read_bytes_header_solidity<B: ByteOrder, const ALIGN: usize>(
    buf: &impl Buf,
    offset: usize,
) -> Result<(usize, usize), CodecError> {
    // Data offset position
    let data_offset = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;

    let data_len = read_u32_aligned::<B, ALIGN>(buf, data_offset + offset)? as usize;

    // Data starts immediately after aligned length
    Ok((data_offset + offset, data_len))
}

pub fn read_bytes_header_solidity2<B: ByteOrder, const ALIGN: usize>(
    buf: &impl Buf,
    offset: usize,
) -> Result<(usize, usize), CodecError> {
    // Offset is relative to the buffer start
    let data_offset = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;

    // Data length is located exactly at `data_offset` from buffer start
    let data_len = read_u32_aligned::<B, ALIGN>(buf, data_offset)? as usize;

    // Data starts immediately after the length field (ALIGN bytes)
    Ok((data_offset, data_len))
}


#[cfg(test)]
pub mod test_utils {
    use crate::optimized::{ctx::EncodingContext, encoder::Encoder, utils::read_u32_aligned};
    use byteorder::{BigEndian, ByteOrder, LittleEndian};
    use bytes::BytesMut;
    use hex::encode;

    pub(crate) fn assert_codec_compact<T>(
        expected_header_hex: &str,
        expected_tail_hex: &str,
        value: &T,
    ) where
        T: Encoder<LittleEndian, 4, false> + PartialEq + std::fmt::Debug + Clone,
    {
        let mut ctx = EncodingContext::default();
        ctx.hdr_size = value.header_size() as u32;
        println!("ctx: {:?}", ctx);

        let mut header_buf = BytesMut::new();
        let w = T::encode_header(value, &mut header_buf, &mut ctx);
        assert!(w.is_ok(), "encode_header failed: {:?}", w);
        println!("ctx after header: {:?}", ctx);

        assert_eq!(
            expected_header_hex,
            encode(&header_buf),
            "header bytes mismatch"
        );

        let mut tail_buf = BytesMut::new();
        let w = T::encode_tail(value, &mut tail_buf, &mut ctx);
        assert!(w.is_ok(), "encode_tail failed: {:?}", w);
        assert_eq!(expected_tail_hex, encode(&tail_buf), "tail bytes mismatch");

        let mut full_buf = header_buf.clone();
        full_buf.extend_from_slice(&tail_buf);
        let decoded = T::decode(&mut &full_buf[..], 0).expect("decode failed");
        assert_eq!(decoded, *value, "decoded value mismatch");
    }
    pub(crate) fn assert_codec_sol<T>(expected_header_hex: &str, expected_tail_hex: &str, value: &T)
    where
        T: Encoder<BigEndian, 32, true> + PartialEq + std::fmt::Debug + Clone,
    {
        let mut ctx = EncodingContext::default();
        ctx.hdr_size = value.header_size() as u32;

        println!("hdr_size: {}", ctx.hdr_size);

        let mut header_buf = BytesMut::new();
        let w = T::encode_header(value, &mut header_buf, &mut ctx);
        assert!(w.is_ok(), "encode_header failed: {:?}", w);

        assert_eq!(
            expected_header_hex,
            encode(&header_buf),
            "header bytes mismatch"
        );

        let mut tail_buf = BytesMut::new();
        let w = T::encode_tail(value, &mut tail_buf, &mut ctx);
        assert!(w.is_ok(), "encode_tail failed: {:?}", w);
        assert_eq!(expected_tail_hex, encode(&tail_buf), "tail bytes mismatch");

        let mut full_buf = header_buf.clone();
        full_buf.extend_from_slice(&tail_buf);
        let decoded = T::decode(&mut &full_buf[..], 0).expect("decode failed");
        assert_eq!(decoded, *value, "decoded value mismatch");
    }

    pub(crate) fn assert_roundtrip_compact<T>(value: &T)
    where
        T: Encoder<LittleEndian, 4, false> + PartialEq + std::fmt::Debug,
    {
        let mut ctx = EncodingContext::default();
        ctx.hdr_size = value.header_size() as u32;

        let mut header_buf = BytesMut::new();
        value
            .encode_header(&mut header_buf, &mut ctx)
            .expect("encode_header failed");
        println!("header");
        print_encoded::<LittleEndian, 4>(&header_buf);

        let mut tail_buf = BytesMut::new();
        value
            .encode_tail(&mut tail_buf, &mut ctx)
            .expect("encode_tail failed");

        println!("tail");
        print_encoded::<LittleEndian, 4>(&tail_buf);

        let mut full_buf = header_buf.clone();
        full_buf.extend_from_slice(&tail_buf);

        let decoded = T::decode(&mut &full_buf[..], 0).expect("decode failed");
        assert_eq!(decoded, *value, "decoded value mismatch");
    }

    pub(crate) fn assert_roundtrip_sol<T>(value: &T)
    where
        T: Encoder<BigEndian, 32, true> + PartialEq + std::fmt::Debug,
    {
        let mut ctx = EncodingContext::default();
        ctx.hdr_size = value.header_size() as u32;

        let mut header_buf = BytesMut::new();
        value
            .encode_header(&mut header_buf, &mut ctx)
            .expect("encode_header failed");

        let mut tail_buf = BytesMut::new();
        value
            .encode_tail(&mut tail_buf, &mut ctx)
            .expect("encode_tail failed");

        let mut full_buf = header_buf.clone();
        full_buf.extend_from_slice(&tail_buf);

        let decoded = T::decode(&mut &full_buf[..], 0).expect("decode failed");
        assert_eq!(decoded, *value, "decoded value mismatch");
    }

    #[allow(dead_code)]
    pub(crate) fn print_encoded<B: ByteOrder, const ALIGN: usize>(buf: impl AsRef<[u8]>) {
        let bytes = buf.as_ref();
        println!("concat!(");
        for (i, chunk) in bytes.chunks_exact(ALIGN).enumerate() {
            let hex_chunk = encode(chunk);
            let decimal_value = read_u32_aligned::<B, ALIGN>(&chunk, 0).unwrap();
            let offset = i * ALIGN;
            println!(
                "    \"{}\", // [0x{:04x}] {} = {}",
                hex_chunk, offset, offset, decimal_value
            );
        }
        println!(");");
    }

    pub fn encode_alloy_sol<T: alloy_sol_types::SolValue>(value: &T) -> Vec<u8> {
        value.abi_encode()
    }

    pub fn decode_alloy_sol<T: alloy_sol_types::SolType>(
        data: &[u8],
    ) -> Result<T::RustType, alloy_sol_types::Error> {
        T::abi_decode(data)
    }
}
