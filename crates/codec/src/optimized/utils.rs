use crate::optimized::error::CodecError;
use byteorder::ByteOrder;
use bytes::{Buf, BufMut};

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
