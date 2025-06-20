use crate::{
    alloc::string::ToString,
    encoder::{align_up, read_u32_aligned, write_u32_aligned},
    error::{CodecError, DecodingError},
};
use byteorder::ByteOrder;
use bytes::{Buf, BufMut, Bytes, BytesMut};
use core::mem;

/// Universal function to write bytes in Solidity or WASM compatible format
///
/// # Parameters
///
/// - `buf`: A mutable reference to a `BytesMut` buffer where the bytes will be written.
/// - `header_offset`: The offset in the buffer where the header should be written.
/// - `data`: A slice of bytes representing the data to be written.
/// - `elements`: The number of elements in the dynamic array.
///
/// # Type Parameters
///
/// - `B`: The byte order to be used (e.g., `BigEndian` or `LittleEndian`).
/// - `ALIGN`: The alignment size.
/// - `SOL_MODE`: A boolean indicating whether to use Solidity mode (`true`) or WASM mode (`false`).
///
/// # Returns
///
/// The number of bytes written, including alignment.
///
/// # Example
///
/// ```
/// use bytes::BytesMut;
/// use byteorder::BigEndian;
/// use fluentbase_codec::bytes_codec::write_bytes;
/// let mut buf = BytesMut::new();
/// let data = &[1, 2, 3, 4, 5];
/// let elements = data.len() as u32;
/// let written = write_bytes::<BigEndian, 32, true>(&mut buf, 0, data, elements);
/// assert_eq!(written, 37);
/// ```
pub fn write_bytes<B, const ALIGN: usize, const SOL_MODE: bool>(
    buf: &mut impl BufMut,
    offset: usize,
    data: &[u8],
    elements: u32, // number of elements in a dynamic array
) -> usize
where
    B: ByteOrder,
{
    if SOL_MODE {
        write_bytes_solidity::<B, ALIGN>(buf, data, elements)
    } else {
        write_bytes_wasm::<B, ALIGN>(buf, offset, data)
    }
}

/// Write bytes in Solidity compatible format
pub fn write_bytes_solidity<B: ByteOrder, const ALIGN: usize>(
    buf: &mut impl BufMut,
    data: &[u8],
    elements: u32, // Number of elements
) -> usize {
    // Write length of the data (number of elements)
    let mut n = write_u32_aligned::<B, ALIGN>(buf, elements);
    // Append the actual data
    buf.put_slice(data);
    n += data.len();
    // Return the number of bytes written (including alignment)
    n
}

/// Write bytes in WASM compatible format
pub fn write_bytes_wasm<B: ByteOrder, const ALIGN: usize>(
    buf: &mut impl BufMut,
    offset: usize,
    data: &[u8],
) -> usize {
    let mut n = 0;
    // Write offset and data size
    n += write_u32_aligned::<B, ALIGN>(buf, offset as u32);
    n += write_u32_aligned::<B, ALIGN>(buf, data.len() as u32);
    // Append the actual data
    buf.put_slice(data);
    n += data.len();
    // Total bytes written
    n
}

pub fn read_bytes<B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool>(
    buf: &impl Buf,
    offset: usize,
) -> Result<Bytes, CodecError> {
    let (data_offset, data_len) = read_bytes_header::<B, ALIGN, SOL_MODE>(buf, offset)?;

    let data = if SOL_MODE {
        buf.chunk()[data_offset + 32..data_offset + 32 + data_len].to_vec()
    } else {
        buf.chunk()[data_offset..data_offset + data_len].to_vec()
    };

    Ok(Bytes::from(data))
}

/// Reads the header of the bytes data in Solidity or WASM compatible format
/// Returns the offset and size of the data
pub fn read_bytes_header<B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool>(
    buf: &impl Buf,
    offset: usize,
) -> Result<(usize, usize), CodecError> {
    match SOL_MODE {
        true => read_bytes_header_solidity::<B, ALIGN>(buf, offset),
        false => read_bytes_header_wasm::<B, ALIGN>(buf, offset),
    }
}

pub fn read_bytes_header_wasm<B: ByteOrder, const ALIGN: usize>(
    buffer: &impl Buf,
    offset: usize,
) -> Result<(usize, usize), CodecError> {
    let aligned_elem_size = align_up::<ALIGN>(mem::size_of::<u32>());

    if buffer.remaining() < offset + aligned_elem_size * 2 {
        return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
            expected: offset + aligned_elem_size * 2,
            found: buffer.remaining(),
            msg: "buffer too small to read bytes header".to_string(),
        }));
    }

    let data_offset = read_u32_aligned::<B, ALIGN>(buffer, offset)? as usize;

    let data_len = read_u32_aligned::<B, ALIGN>(buffer, offset + aligned_elem_size)? as usize;

    Ok((data_offset, data_len))
}

/// Reads the header of a Solidiof the data
///
/// Given the original data:
/// ```
/// let original: Vec<Vec<u32>> = vec![vec![1, 2, 3], vec![4, 5]];
/// ```
/// The Solidity encoding would look like this:
///
///
/// 000 000  : 00 00 00 20   ||  032 |
/// 032 000  : 00 00 00 02   ||  002 |
///
/// 064 000  : 00 00 00 40   ||  064 | <---- buf should start here
/// 096 032  : 00 00 00 c0   ||  192 |
/// 128 064  : 00 00 00 03   ||  003 |
/// 160 096  : 00 00 00 01   ||  001 |
/// 192 128  : 00 00 00 02   ||  002 |
/// 224 160  : 00 00 00 03   ||  003 |
/// 256 192  : 00 00 00 02   ||  002 |
/// 288 224  : 00 00 00 04   ||  004 |
/// 320 256  : 00 00 00 05   ||  005 |
///
///
/// # Parameters
///
/// - `buf`: The buffer containing the encoded data.
/// - `offset`: The offset from which to start reading.
///
/// # Returns
///
/// A tuple containing the data offset and the data length.
///
/// # Errors
///
/// Returns a `CodecError` if reading from the buffer fails.
pub fn read_bytes_header_solidity<B: ByteOrder, const ALIGN: usize>(
    buf: &impl Buf,
    offset: usize,
) -> Result<(usize, usize), CodecError> {
    let data_offset = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;

    let data_len = read_u32_aligned::<B, ALIGN>(buf, data_offset)? as usize;

    Ok((data_offset, data_len))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoder::{CompactABI, SolidityABI};
    use alloy_sol_types::{sol_data, SolType};
    use byteorder::{BigEndian, LE};

    #[test]
    fn test_write_bytes_sol() {
        let mut buf = BytesMut::new();

        // For byte slice
        let bytes: &[u8] = &[1, 2, 3, 4, 5];
        let written = write_bytes_solidity::<BigEndian, 32>(&mut buf, bytes, bytes.len() as u32);
        assert_eq!(written, 37); // length (32) + (data + padding) (32)
        let expected = [
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 5, 1, 2, 3, 4, 5,
        ];

        assert_eq!(buf.to_vec(), expected);
        let mut buf = BytesMut::new();

        // For Vec<u32>

        let vec_u32 = [0u8, 0, 0, 10, 0, 0, 0, 20, 0, 0, 0, 30];

        let written = write_bytes_solidity::<BigEndian, 32>(&mut buf, &vec_u32, 3);
        assert_eq!(written, 44); // length (32) + data

        let expected = [
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 3, 0, 0, 0, 10, 0, 0, 0, 20, 0, 0, 0, 30,
        ];
        assert_eq!(buf.to_vec(), expected);
    }

    #[test]
    fn test_read_bytes_header_solidity_simple() {
        let original = alloy_primitives::Bytes::from(vec![1, 2, 3, 4, 5]);

        let mut buf = BytesMut::new();
        SolidityABI::encode(&original, &mut buf).unwrap();

        let encoded = buf.freeze();

        let encoded_alloy = &sol_data::Bytes::abi_encode(&original)[..];

        println!("alloy encoded: {:?}", encoded_alloy);
        println!("encoded: {:?}", hex::encode(&encoded));
        let (offset, size) = read_bytes_header::<BigEndian, 32, true>(&encoded, 0).unwrap();

        println!("Offset: {}, Size: {}", offset, size);

        assert_eq!(offset, 32);
        assert_eq!(size, 5);
    }

    #[test]
    fn test_read_bytes_header_solidity_complex() {
        let original: Vec<Vec<u32>> = vec![vec![1, 2, 3], vec![4, 5]];

        let mut buf = BytesMut::new();
        SolidityABI::encode(&original, &mut buf).unwrap();

        let encoded = buf.freeze();
        println!("encoded: {:?}", hex::encode(&encoded));

        let chunk = &encoded.chunk()[64..];

        // 1st vec
        let (offset, size) = read_bytes_header_solidity::<BigEndian, 32>(&chunk, 0).unwrap();
        assert_eq!(offset, 64);
        assert_eq!(size, 3);

        // 2nd vec
        let (offset, size) = read_bytes_header_solidity::<BigEndian, 32>(&chunk, 32).unwrap();
        assert_eq!(offset, 192);
        assert_eq!(size, 2);
    }

    #[test]
    fn test_read_bytes_header_wasm() {
        let original = alloy_primitives::Bytes::from(vec![1, 2, 3, 4, 5]);

        let mut buf = BytesMut::new();
        CompactABI::encode(&original, &mut buf).unwrap();

        let encoded = buf.freeze();
        println!("encoded: {:?}", hex::encode(&encoded));

        let (offset, size) = read_bytes_header::<LE, 4, false>(&encoded, 0).unwrap();

        println!("Offset: {}, Size: {}", offset, size);

        assert_eq!(offset, 8);
        assert_eq!(size, 5);
    }
}
