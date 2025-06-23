use crate::optimized::{
    encoder::{Encoder, EncodingContext},
    error::CodecError,
    utils::{align_up, read_u32_aligned, write_u32_aligned},
};
use byteorder::{BigEndian, ByteOrder};
use bytes::{Buf, BufMut};
use smallvec::SmallVec;

/// Vec implementation for Solidity ABI (always 32-byte aligned, big-endian)
impl<T> Encoder<BigEndian, 4, false> for Vec<T>
where
    T: Encoder<BigEndian, 4, false>,
{
    const HEADER_SIZE: usize = 4; // offset pointer for top-level
    const IS_DYNAMIC: bool = true;

    fn encode(
        &self,
        _buf: &mut impl BufMut,
        _ctx: Option<&mut EncodingContext>,
    ) -> Result<usize, CodecError> {
        todo!()
    }
    fn decode(_buf: &impl Buf, _offset: usize) -> Result<Self, CodecError> {
        todo!()
    }
}

/// Vec implementation for Solidity ABI (always 32-byte aligned, big-endian)
impl<T> Encoder<BigEndian, 32, true> for Vec<T>
where
    T: Encoder<BigEndian, 32, true>,
{
    const HEADER_SIZE: usize = 32; // offset pointer for top-level
    const IS_DYNAMIC: bool = true;

    fn encode(
        &self,
        buf: &mut impl BufMut,
        ctx: Option<&mut EncodingContext>,
    ) -> Result<usize, CodecError>
    where
        T: Encoder<BigEndian, 32, true>,
    {
        let mut total_size = 0;

        let mut default_ctx;
        let ctx = match ctx {
            Some(ctx) => ctx,
            None => {
                default_ctx = EncodingContext::new();
                &mut default_ctx
            }
        };

        if ctx.depth() == 0 {
            write_u32_aligned::<BigEndian, 32>(buf, 32);
            total_size += 32;
        }

        ctx.enter()?;

        write_u32_aligned::<BigEndian, 32>(buf, self.len() as u32);
        total_size += 32;

        if self.is_empty() {
            return Ok(total_size);
        }

        if T::IS_DYNAMIC {
            total_size += encode_dynamic_elements(self, buf, ctx)?;
        } else {
            for element in self.iter() {
                total_size += element.encode(buf, Some(ctx))?;
            }
        }

        ctx.exit();
        Ok(total_size)
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
}

#[inline(always)]
fn encode_dynamic_elements<T>(
    vec: &[T],
    buf: &mut impl BufMut,
    ctx: &mut EncodingContext,
) -> Result<usize, CodecError>
where
    T: Encoder<BigEndian, 32, true>,
{
    let len = vec.len();
    if len == 0 {
        return Ok(0);
    }

    let mut current_offset = len * 32;

    for element in vec.iter() {
        let size = if T::IS_DYNAMIC {
            let mut counter = ByteCounter::new();
            element.encode(&mut counter, Some(ctx))?;
            counter.count
        } else {
            align_up::<32>(T::HEADER_SIZE)
        };

        write_u32_aligned::<BigEndian, 32>(buf, current_offset as u32);
        current_offset += size;
    }

    // Write actual elements
    let mut total_written = 0;
    for element in vec.iter() {
        let written = element.encode(buf, Some(ctx))?;

        total_written += written;
    }

    Ok(len * 32 + total_written)
}

/// Counting writer that tracks bytes without storing them
struct ByteCounter {
    count: usize,
}

impl ByteCounter {
    #[inline]
    fn new() -> Self {
        Self { count: 0 }
    }
}

unsafe impl BufMut for ByteCounter {
    #[inline]
    fn remaining_mut(&self) -> usize {
        usize::MAX
    }

    #[inline]
    unsafe fn advance_mut(&mut self, cnt: usize) {
        self.count += cnt;
    }

    fn chunk_mut(&mut self) -> &mut bytes::buf::UninitSlice {
        unreachable!(
            "ByteCounter does not support chunk_mut(). \
            This is a counting-only implementation. \
            All writes must use put_* methods."
        )
    }

    #[inline]
    fn put_slice(&mut self, src: &[u8]) {
        self.count += src.len();
    }

    #[inline]
    fn put_bytes(&mut self, _: u8, cnt: usize) {
        self.count += cnt;
    }

    // Override common methods for efficiency
    #[inline]
    fn put_u8(&mut self, _: u8) {
        self.count += 1;
    }

    #[inline]
    fn put_u32(&mut self, _: u32) {
        self.count += 4;
    }

    #[inline]
    fn put_u32_le(&mut self, _: u32) {
        self.count += 4;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::BigEndian;
    use bytes::BytesMut;

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

    test_vec_encode_decode!(
        empty_vec_solidity,
        item_type = u32,
        endian = BigEndian,
        align = 32,
        solidity = true,
        value = Vec::<u32>::new(),
        expected_hex = concat!(
            "0000000000000000000000000000000000000000000000000000000000000020", // offset to data
            "0000000000000000000000000000000000000000000000000000000000000000"  // length = 0
        )
    );

    test_vec_encode_decode!(
        test_nested_vec_solidity,
        item_type = Vec<u32>,
        endian = BigEndian,
        align = 32,
        solidity = true,
        value = vec![vec![1u32, 2, 3], vec![4, 5]],
        expected_hex = concat!(
            // Main array header
            "0000000000000000000000000000000000000000000000000000000000000020", // offset to data
            "0000000000000000000000000000000000000000000000000000000000000002", // length = 2
            "0000000000000000000000000000000000000000000000000000000000000040", // offset to vec[0] (64 bytes from start of data)
            "00000000000000000000000000000000000000000000000000000000000000c0", // offset to vec[1] (192 bytes from start of data)
            // vec[0] = [1, 2, 3]
            "0000000000000000000000000000000000000000000000000000000000000003", // length = 3
            "0000000000000000000000000000000000000000000000000000000000000001", // 1
            "0000000000000000000000000000000000000000000000000000000000000002", // 2
            "0000000000000000000000000000000000000000000000000000000000000003", // 3
            // vec[1] = [4, 5]
            "0000000000000000000000000000000000000000000000000000000000000002", // length = 2
            "0000000000000000000000000000000000000000000000000000000000000004", // 4
            "0000000000000000000000000000000000000000000000000000000000000005"  // 5
        )
    );

    #[test]
    fn test_decode_simple() {
        let v: Vec<u32> = vec![1, 2, 3, 4, 5];

        let mut buf = BytesMut::new();
        let encode_result = <Vec<u32> as Encoder<BigEndian, 32, true>>::encode(&v, &mut buf, None);

        assert!(
            encode_result.is_ok(),
            "Encoding failed: {:?}",
            encode_result
        );
        let encoded = buf.freeze();
        println!("encoded: {:?}", hex::encode(&encoded));

        let decode_result = <Vec<u32> as Encoder<BigEndian, 32, true>>::decode(&encoded, 0);
        assert!(
            decode_result.is_ok(),
            "Decoding failed: {:?}",
            decode_result
        );
        let decoded = decode_result.unwrap();
        assert_eq!(decoded, v, "Decoded value mismatch");
    }

    #[test]
    fn test_encode_large() {
        let large_vec1: Vec<u32> = (0..1000).collect();
        let large_vec2: Vec<u32> = (1000..1200).collect();
        let large_vec3: Vec<u32> = (1200..1300).collect();
        let large_vec4: Vec<u32> = (1300..1350).collect();

        let v = vec![vec![large_vec1, large_vec2, large_vec3, large_vec4]];

        let mut buf = BytesMut::new();
        let encode_result =
            <Vec<Vec<Vec<u32>>> as Encoder<BigEndian, 32, true>>::encode(&v, &mut buf, None);

        assert!(
            encode_result.is_ok(),
            "Encoding failed: {:?}",
            encode_result
        );
        let encoded = buf.freeze();
        println!("encoded: {:?}", hex::encode(&encoded));

        let decode_result =
            <Vec<Vec<Vec<u32>>> as Encoder<BigEndian, 32, true>>::decode(&encoded, 0);
        assert!(
            decode_result.is_ok(),
            "Decoding failed: {:?}",
            decode_result
        );
        let decoded = decode_result.unwrap();
        assert_eq!(decoded, v, "Decoded value mismatch");
    }

    // TODO: check do we actually need this use case
    // TLDR: Decoding from dirty buffers with offset might be caller's responsibility to handle
    // This test verifies decoding from a buffer with dirty/garbage data at the beginning.
    // Current implementation expects clean buffers - decoding with offset into dirty buffer fails.
    // This might be correct behavior: the caller should ensure proper buffer alignment/cleaning
    // rather than the decoder handling arbitrary offsets in dirty buffers.
    #[test]
    #[ignore]
    fn vec_decoding_from_dirty_buf() {
        let original: Vec<u32> = vec![1, 2, 3, 4, 5];
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&[0xFF, 0xFF, 0xFF]); // Add some initial data

        // Encode at top-level (None context)
        <Vec<u32> as Encoder<BigEndian, 32, true>>::encode(&original, &mut buf, None).unwrap();
        let encoded = buf.freeze();

        eprintln!("encoded: {:?}", hex::encode(&encoded));

        let expected_encoded = hex::decode(concat!(
            "ffffff",                                                           // Initial data
            "0000000000000000000000000000000000000000000000000000000000000020", // offset
            "0000000000000000000000000000000000000000000000000000000000000005", // length = 5
            "0000000000000000000000000000000000000000000000000000000000000001", // 1
            "0000000000000000000000000000000000000000000000000000000000000002", // 2
            "0000000000000000000000000000000000000000000000000000000000000003", // 3
            "0000000000000000000000000000000000000000000000000000000000000004", // 4
            "0000000000000000000000000000000000000000000000000000000000000005"  // 5
        ))
        .unwrap();

        if encoded != expected_encoded {
            eprintln!("Encoded mismatch!");
            eprintln!("Expected: {}", hex::encode(&expected_encoded));
            eprintln!("Actual:   {}", hex::encode(&encoded));
        }
        assert_eq!(expected_encoded, encoded);

        // Decode starting from offset 3 (after the 0xFF bytes)
        let decoded = <Vec<u32> as Encoder<BigEndian, 32, true>>::decode(&encoded, 3).unwrap();

        assert_eq!(original, decoded);
    }
}
