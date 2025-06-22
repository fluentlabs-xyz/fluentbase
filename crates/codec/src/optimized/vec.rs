use crate::optimized::{
    encoder::{Encoder, EncodingContext},
    error::CodecError,
    utils::{align_up, is_big_endian, read_u32_aligned, write_u32_aligned},
};
use byteorder::{BigEndian, ByteOrder};
use bytes::{Buf, BufMut};

/// Vec implementation for Solidity ABI (always 32-byte aligned, big-endian)
impl<T> Encoder<BigEndian, 4, false> for Vec<T>
where
    T: Encoder<BigEndian, 4, false>,
{
    const HEADER_SIZE: usize = 4; // offset pointer for top-level
    const IS_DYNAMIC: bool = true;

    fn encode(
        &self,
        buf: &mut impl BufMut,
        ctx: Option<&mut EncodingContext>,
    ) -> Result<usize, CodecError> {
        todo!()
    }
    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
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
    ) -> Result<usize, CodecError> {
        let mut total_size = 0;

        // Top-level dynamic types require an offset pointer
        let is_top_level = ctx.is_none();
        if is_top_level {
            write_u32_aligned::<BigEndian, 32>(buf, 32);
            total_size += 32;
        }

        // Write array length
        write_u32_aligned::<BigEndian, 32>(buf, self.len() as u32);
        total_size += 32;

        if self.is_empty() {
            return Ok(total_size);
        }

        // Create context for nested elements if needed
        let mut local_ctx = EncodingContext::new();
        let ctx_for_nested = ctx.unwrap_or(&mut local_ctx);

        if T::IS_DYNAMIC {
            // Two-pass encoding for dynamic elements
            total_size += encode_dynamic_elements(self, buf, ctx_for_nested)?;
        } else {
            // Single-pass encoding for static elements
            total_size += encode_static_elements(self, buf, ctx_for_nested)?;
        }

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

/// Single-pass encoding for static elements
fn encode_static_elements<T>(
    vec: &[T],
    buf: &mut impl BufMut,
    ctx: &mut EncodingContext,
) -> Result<usize, CodecError>
where
    T: Encoder<BigEndian, 32, true>,
{
    let mut size = 0;

    for element in vec {
        size += element.encode(buf, Some(ctx))?;
    }

    Ok(size)
}

/// Two-pass encoding for dynamic elements
fn encode_dynamic_elements<T>(
    vec: &[T],
    buf: &mut impl BufMut,
    ctx: &mut EncodingContext,
) -> Result<usize, CodecError>
where
    T: Encoder<BigEndian, 32, true>,
{
    // Small vectors optimization threshold
    const SMALL_VEC_THRESHOLD: usize = 16;

    // PASS 1: Calculate sizes
    let sizes = if vec.len() <= SMALL_VEC_THRESHOLD {
        // Stack allocation for small vectors
        let mut stack_sizes = [0usize; SMALL_VEC_THRESHOLD];

        for (i, element) in vec.iter().enumerate() {
            let mut counter = ByteCounter::new();
            element.encode(&mut counter, Some(ctx))?;
            stack_sizes[i] = counter.count;
        }

        stack_sizes[..vec.len()].to_vec()
    } else {
        // Heap allocation for large vectors
        vec.iter()
            .map(|element| {
                let mut counter = ByteCounter::new();
                element.encode(&mut counter, Some(ctx))?;
                Ok(counter.count)
            })
            .collect::<Result<Vec<_>, CodecError>>()?
    };

    // Calculate and write offsets
    let offsets_size = vec.len() * 32;

    // First element starts after length field + all offsets
    let mut element_position = 32 + offsets_size;

    for &size in &sizes {
        // Offsets are relative to the start of array data (where length is stored)
        // So we subtract 32 (length field size) from absolute position
        write_u32_aligned::<BigEndian, 32>(buf, (element_position - 32) as u32);
        element_position += size;
    }

    // PASS 2: Write actual element data
    let mut total_size = offsets_size;

    for (i, element) in vec.iter().enumerate() {
        let written = element.encode(buf, Some(ctx))?;

        // Verify size consistency in debug mode
        #[cfg(debug_assertions)]
        {
            if written != sizes[i] {
                return Err(CodecError::InvalidData(
                    "size mismatch between calculation and actual encoding",
                ));
            }
        }

        total_size += written;
    }

    Ok(total_size)
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

    #[inline]
    fn bytes_written(&self) -> usize {
        self.count
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

    // Override common methods for efficiency
    #[inline]
    fn put_u8(&mut self, _: u8) {
        self.count += 1;
    }

    #[inline]
    fn put_slice(&mut self, src: &[u8]) {
        self.count += src.len();
    }

    #[inline]
    fn put_u32(&mut self, _: u32) {
        self.count += 4;
    }

    #[inline]
    fn put_u32_le(&mut self, _: u32) {
        self.count += 4;
    }

    #[inline]
    fn put_bytes(&mut self, _: u8, cnt: usize) {
        self.count += cnt;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::{BigEndian, LittleEndian};
    use bytes::{Bytes, BytesMut};

    /// Macro for testing a standard encode -> decode roundtrip against a hex snapshot.
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
        nested_vec_solidity,
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
    fn vec_encoding_with_offset() {
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

    #[test]
    fn test_read_u32_aligned() {
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

        let buf = Bytes::from(expected_encoded);

        let chunk = buf.chunk();
        println!("chunk len: {:?}", chunk.remaining());

        // // let trimmed_chunk = &chunk[3..];
        // println!("trimmed_chunk: {:?}", trimmed_chunk);
        // // let offset =

        let value = read_u32_aligned::<BigEndian, 32>(&buf, 3).unwrap();
        println!("aligned: {:?}", value);
    }
}
