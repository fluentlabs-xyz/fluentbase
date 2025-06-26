use crate::{
    alloc::string::ToString,
    optimized::{encoder::Encoder, error::CodecError},
};
use byteorder::ByteOrder;
use bytes::{Buf, BufMut};

/// Checks if the given byte order is big-endian.
pub fn is_big_endian<B: ByteOrder>() -> bool {
    B::read_u16(&[0x12, 0x34]) == 0x1234
}

/// Rounds up the given offset to the nearest multiple of ALIGN.
/// ALIGN must be a power of two.
#[inline]
pub const fn align_up<const ALIGN: usize>(offset: usize) -> usize {
    (offset + ALIGN - 1) & !(ALIGN - 1)
}

pub fn write_u32_aligned<B: ByteOrder, const ALIGN: usize>(buf: &mut impl BufMut, value: u32) {
    let aligned_value_size = align_up::<ALIGN>(4);

    if is_big_endian::<B>() {
        // For big-endian, add padding first, then write value
        let padding = aligned_value_size - 4;
        if padding > 0 {
            buf.put_bytes(0, padding);
        }
        buf.put_u32(value);
    } else {
        // For little-endian, write value first, then add padding
        buf.put_u32_le(value);
        let padding = aligned_value_size - 4;
        if padding > 0 {
            buf.put_bytes(0, padding);
        }
    }
}
pub fn read_u32_aligned<B: ByteOrder, const ALIGN: usize>(
    buf: &impl Buf,
    offset: usize,
) -> Result<u32, CodecError> {
    let word_size = align_up::<ALIGN>(4);

    let end = offset + word_size;

    if buf.remaining() < end {
        println!(
            "read_u32_aligned>>>Buffer too small : expected {}, actual {}",
            end,
            buf.remaining()
        );
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

#[cfg(test)]
pub(crate) mod tests {
    use crate::optimized::{ctx::EncodingContext, encoder::Encoder};
    use byteorder::{BigEndian, LittleEndian};
    use bytes::BytesMut;

    pub fn assert_codec<T>(value: &T, expected_header_hex: &str, expected_tail_hex: &str)
    where
        T: Encoder<LittleEndian, 4, false, Ctx = EncodingContext>
            + PartialEq
            + std::fmt::Debug
            + Clone,
    {
        // Header
        let mut ctx = EncodingContext::new();
        let _ = T::header_size(value, &mut ctx);
        ctx.data_ptr = ctx.hdr_ptr;

        let mut header_buf = BytesMut::new();
        let w = T::encode_header(value, &mut header_buf, &mut ctx);
        assert!(w.is_ok(), "encode_header failed: {:?}", w);
        assert_eq!(
            expected_header_hex,
            hex::encode(header_buf.clone()),
            "header bytes mismatch"
        );

        // Tail
        let mut tail_buf = BytesMut::new();
        let w = T::encode_tail(value, &mut tail_buf);
        assert!(w.is_ok(), "encode_tail failed: {:?}", w);
        assert_eq!(
            expected_tail_hex,
            hex::encode(tail_buf.clone()),
            "tail bytes mismatch"
        );

        // Decode header+tail
        let mut full_buf = header_buf.clone();
        full_buf.extend_from_slice(&tail_buf);
        let decoded = T::decode(&mut &full_buf[..], 0).expect("decode failed");
        assert_eq!(decoded, *value, "decoded value mismatch");
    }

    pub fn assert_solidity_codec<T>(
        value: &T,
        expected_hex: &str,
    ) where
        T: Encoder<BigEndian, 32, true> + PartialEq + std::fmt::Debug + Clone,
    {
        use bytes::BytesMut;
        // Encode
        let mut buf = BytesMut::new();
        let w = T::encode(value, &mut buf, None);
        assert!(w.is_ok(), "Solidity encode failed: {:?}", w);

        assert_eq!(
            expected_hex,
            hex::encode(buf.clone()),
            "Solidity ABI encoded bytes mismatch"
        );

        // Decode
        let decoded = T::decode(&buf, 0).expect("Solidity decode failed");
        assert_eq!(*value, decoded, "Solidity roundtrip value mismatch");
    }

}
