use alloc::vec::Vec;
use byteorder::{ByteOrder, LittleEndian};
use paste::paste;

pub trait WritableBuffer {
    fn write_i8(&mut self, field_offset: usize, value: i8) -> usize;
    fn write_u8(&mut self, field_offset: usize, value: u8) -> usize;
    fn write_i16(&mut self, field_offset: usize, value: i16) -> usize;
    fn write_u16(&mut self, field_offset: usize, value: u16) -> usize;
    fn write_i32(&mut self, field_offset: usize, value: i32) -> usize;
    fn write_u32(&mut self, field_offset: usize, value: u32) -> usize;
    fn write_i64(&mut self, field_offset: usize, value: i64) -> usize;
    fn write_u64(&mut self, field_offset: usize, value: u64) -> usize;
    fn write_bytes(&mut self, field_offset: usize, bytes: &[u8]) -> usize;
}

macro_rules! encode_le_int {
    ($typ:ty) => {
        paste! {
            fn [<write_ $typ>](&mut self, field_offset: usize, value: $typ) -> usize {
                LittleEndian::[<write_ $typ>](&mut self.buffer[field_offset..], value);
                core::mem::size_of::<$typ>()
            }
        }
    };
}

pub struct FixedEncoder<const N: usize> {
    header_length: usize,
    body_length: usize,
    buffer: [u8; N],
}

impl<const N: usize> FixedEncoder<N> {
    pub fn new(header_length: usize) -> Self {
        Self {
            header_length,
            body_length: 0,
            buffer: [0; N],
        }
    }

    #[allow(dead_code)]
    pub fn bytes(&self) -> &[u8] {
        &self.buffer[..(self.header_length + self.body_length)]
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.header_length + self.body_length
    }

    #[allow(dead_code)]
    pub fn finalize(self) -> ([u8; N], usize) {
        (self.buffer, self.len())
    }
}

impl<const N: usize> WritableBuffer for FixedEncoder<N> {
    fn write_i8(&mut self, field_offset: usize, value: i8) -> usize {
        self.buffer[field_offset] = value as u8;
        1
    }
    fn write_u8(&mut self, field_offset: usize, value: u8) -> usize {
        self.buffer[field_offset] = value;
        1
    }

    encode_le_int!(u16);
    encode_le_int!(i16);
    encode_le_int!(u32);
    encode_le_int!(i32);
    encode_le_int!(u64);
    encode_le_int!(i64);

    fn write_bytes(&mut self, field_offset: usize, bytes: &[u8]) -> usize {
        let data_offset = self.len();
        let data_length = bytes.len();
        // write header with data offset and length
        self.write_u32(field_offset + 0, data_offset as u32);
        self.write_u32(field_offset + 4, data_length as u32);
        // write bytes to the end of the buffer
        self.buffer[data_offset..(data_offset + data_length)].copy_from_slice(bytes);
        self.body_length += bytes.len();
        8
    }
}

#[derive(Default)]
pub struct BufferEncoder {
    buffer: Vec<u8>,
}

impl BufferEncoder {
    pub fn new(header_length: usize, data_length: Option<usize>) -> Self {
        let mut buffer = Vec::with_capacity(header_length + data_length.unwrap_or(0));
        buffer.resize(header_length, 0);
        Self { buffer }
    }

    pub fn finalize(self) -> Vec<u8> {
        self.buffer
    }
}

impl WritableBuffer for BufferEncoder {
    fn write_i8(&mut self, field_offset: usize, value: i8) -> usize {
        self.buffer[field_offset] = value as u8;
        1
    }
    fn write_u8(&mut self, field_offset: usize, value: u8) -> usize {
        self.buffer[field_offset] = value;
        1
    }

    encode_le_int!(u16);
    encode_le_int!(i16);
    encode_le_int!(u32);
    encode_le_int!(i32);
    encode_le_int!(u64);
    encode_le_int!(i64);

    fn write_bytes(&mut self, field_offset: usize, bytes: &[u8]) -> usize {
        let data_offset = self.buffer.len();
        let data_length = bytes.len();
        // write header with data offset and length
        self.write_u32(field_offset + 0, data_offset as u32);
        self.write_u32(field_offset + 4, data_length as u32);
        // write bytes to the end of the buffer
        self.buffer.extend(bytes);
        8
    }
}

#[derive(Default)]
pub struct BufferDecoder<'a> {
    buffer: &'a [u8],
}

macro_rules! decode_le_int {
    ($typ:ty) => {
        paste! {
            pub fn [<read_ $typ>](&self, field_offset: usize) -> $typ {
                LittleEndian::[<read_ $typ>](&self.buffer[field_offset..])
            }
        }
    };
}

impl<'a> BufferDecoder<'a> {
    pub fn new(input: &'a [u8]) -> Self {
        Self { buffer: input }
    }

    pub fn read_i8(&mut self, field_offset: usize) -> i8 {
        self.buffer[field_offset] as i8
    }
    pub fn read_u8(&mut self, field_offset: usize) -> u8 {
        self.buffer[field_offset]
    }

    decode_le_int!(i16);
    decode_le_int!(u16);
    decode_le_int!(i32);
    decode_le_int!(u32);
    decode_le_int!(i64);
    decode_le_int!(u64);

    pub fn read_bytes_header(&self, field_offset: usize) -> (usize, usize) {
        let bytes_offset = self.read_u32(field_offset + 0) as usize;
        let bytes_length = self.read_u32(field_offset + 4) as usize;
        (bytes_offset, bytes_length)
    }

    pub fn read_bytes(&self, field_offset: usize) -> &[u8] {
        let (bytes_offset, bytes_length) = self.read_bytes_header(field_offset);
        &self.buffer[bytes_offset..(bytes_offset + bytes_length)]
    }

    pub fn read_bytes2(&self, field1_offset: usize, field2_offset: usize) -> (&[u8], &[u8]) {
        (
            self.read_bytes(field1_offset),
            self.read_bytes(field2_offset),
        )
    }
}

#[cfg(test)]
mod test {
    use crate::buffer::{BufferDecoder, BufferEncoder, FixedEncoder, WritableBuffer};

    #[test]
    fn test_simple_encoding() {
        struct Test {
            a: u32,
            b: u16,
            c: u64,
        }
        let test = Test {
            a: 100,
            b: 20,
            c: 3,
        };
        let buffer = {
            let mut buffer = BufferEncoder::new(4 + 2 + 8, None);
            let mut offset = 0;
            offset += buffer.write_u32(offset, test.a);
            offset += buffer.write_u16(offset, test.b);
            buffer.write_u64(offset, test.c);
            buffer.finalize()
        };
        println!("{}", hex::encode(&buffer));
        let mut decoder = BufferDecoder::new(buffer.as_slice());
        assert_eq!(decoder.read_u32(0), 100);
        assert_eq!(decoder.read_u16(4), 20);
        assert_eq!(decoder.read_u64(6), 3);
    }

    #[test]
    fn test_fixed_encoding() {
        struct Test {
            a: u32,
            b: u16,
            c: u64,
        }
        let test = Test {
            a: 100,
            b: 20,
            c: 3,
        };
        let buffer = {
            let mut buffer = FixedEncoder::<1024>::new(4 + 2 + 8);
            let mut offset = 0;
            offset += buffer.write_u32(offset, test.a);
            offset += buffer.write_u16(offset, test.b);
            buffer.write_u64(offset, test.c);
            buffer.bytes().to_vec()
        };
        println!("{}", hex::encode(&buffer));
        let mut decoder = BufferDecoder::new(&buffer);
        assert_eq!(decoder.read_u32(0), 100);
        assert_eq!(decoder.read_u16(4), 20);
        assert_eq!(decoder.read_u64(6), 3);
    }

    #[test]
    fn test_fixed_array() {
        let buffer = {
            let mut buffer = FixedEncoder::<1024>::new(4 + 8 + 4 + 8 + 4);
            buffer.write_u32(0, 0xbadcab1e);
            buffer.write_bytes(4, &[0, 1, 2, 3, 4]);
            buffer.write_u32(12, 0xdeadbeef);
            buffer.write_bytes(16, &[5, 6, 7, 8, 9]);
            buffer.write_u32(24, 0x7f);
            buffer.bytes().to_vec()
        };
        println!("{}", hex::encode(&buffer));
        let mut decoder = BufferDecoder::new(buffer.as_slice());
        assert_eq!(decoder.read_u32(0), 0xbadcab1e);
        assert_eq!(decoder.read_bytes(4).to_vec(), vec![0, 1, 2, 3, 4]);
        assert_eq!(decoder.read_u32(12), 0xdeadbeef);
        assert_eq!(decoder.read_bytes(16).to_vec(), vec![5, 6, 7, 8, 9]);
        assert_eq!(decoder.read_u32(24), 0x7f);
    }

    #[test]
    fn test_bytes_array() {
        let buffer = {
            let mut buffer = BufferEncoder::new(4 + 8 + 4 + 8 + 4, None);
            buffer.write_u32(0, 0xbadcab1e);
            buffer.write_bytes(4, &[0, 1, 2, 3, 4]);
            buffer.write_u32(12, 0xdeadbeef);
            buffer.write_bytes(16, &[5, 6, 7, 8, 9]);
            buffer.write_u32(24, 0x7f);
            buffer.finalize()
        };
        println!("{}", hex::encode(&buffer));
        let mut decoder = BufferDecoder::new(buffer.as_slice());
        assert_eq!(decoder.read_u32(0), 0xbadcab1e);
        assert_eq!(decoder.read_bytes(4).to_vec(), vec![0, 1, 2, 3, 4]);
        assert_eq!(decoder.read_u32(12), 0xdeadbeef);
        assert_eq!(decoder.read_bytes(16).to_vec(), vec![5, 6, 7, 8, 9]);
        assert_eq!(decoder.read_u32(24), 0x7f);
    }
}
