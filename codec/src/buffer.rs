use alloc::vec::Vec;
use alloy_primitives::{Address, U160, U256};
use byteorder::{ByteOrder, LittleEndian};

#[derive(Default)]
pub struct BufferEncoder {
    header_length: usize,
    buffer: Vec<u8>,
}

macro_rules! encode_le_int {
    ($typ:ty, $write_fn:ident) => {
        pub fn $write_fn(&mut self, field_offset: usize, value: $typ) -> usize {
            let offset = self.check_header_cap(field_offset, core::mem::size_of::<$typ>());
            LittleEndian::$write_fn(&mut self.buffer[offset..], value);
            core::mem::size_of::<$typ>()
        }
    };
}
macro_rules! encode_le_bigint {
    ($typ:ty, $write_fn:ident) => {
        pub fn $write_fn(&mut self, field_offset: usize, value: &$typ) -> usize {
            let offset = self.check_header_cap(field_offset, <$typ>::BITS / 8);
            self.buffer[offset..].copy_from_slice(value.as_le_slice());
            <$typ>::BITS / 8
        }
    };
}

impl BufferEncoder {
    pub fn new(header_length: usize, data_length: Option<usize>) -> Self {
        let mut buffer = Vec::with_capacity(header_length + data_length.unwrap_or(0));
        buffer.resize(header_length, 0);
        Self {
            header_length,
            buffer,
        }
    }

    pub fn write_i8(&mut self, field_offset: usize, value: i8) -> usize {
        let offset = self.check_header_cap(field_offset, 1);
        self.buffer[offset] = value as u8;
        1
    }
    pub fn write_u8(&mut self, field_offset: usize, value: u8) -> usize {
        let offset = self.check_header_cap(field_offset, 1);
        self.buffer[offset] = value;
        1
    }

    encode_le_int!(u16, write_u16);
    encode_le_int!(i16, write_i16);
    encode_le_int!(u32, write_u32);
    encode_le_int!(i32, write_i32);
    encode_le_int!(u64, write_u64);
    encode_le_int!(i64, write_i64);

    pub fn write_address(&mut self, field_offset: usize, address: &Address) -> usize {
        let offset = self.check_header_cap(field_offset, Address::len_bytes());
        self.buffer[offset..].copy_from_slice(address.as_slice());
        Address::len_bytes()
    }

    encode_le_bigint!(U160, write_u160);
    encode_le_bigint!(U256, write_u256);

    pub fn write_bytes(&mut self, field_offset: usize, bytes: &[u8]) -> usize {
        let data_offset = self.buffer.len();
        let data_length = bytes.len();
        // write header with data offset and length
        self.write_u32(field_offset + 0, data_offset as u32);
        self.write_u32(field_offset + 4, data_length as u32);
        // write bytes to the end of the buffer
        self.buffer.extend(bytes);
        8
    }

    pub fn finalize(self) -> Vec<u8> {
        self.buffer
    }

    fn check_header_cap(&mut self, field_offset: usize, length: usize) -> usize {
        if field_offset + length > self.header_length {
            panic!("header overflow")
        }
        field_offset
    }
}

#[derive(Default)]
pub struct BufferDecoder<'a> {
    buffer: &'a [u8],
}

macro_rules! decode_le_int {
    ($typ:ty, $fn_name:ident) => {
        pub fn $fn_name(&mut self, field_offset: usize) -> $typ {
            LittleEndian::$fn_name(&self.buffer[field_offset..])
        }
    };
}
macro_rules! decode_le_bigint {
    ($typ:ty, $fn_name:ident) => {
        pub fn $fn_name(&mut self, field_offset: usize) -> $typ {
            let mut result = <$typ>::default();
            unsafe {
                result
                    .as_le_slice_mut()
                    .copy_from_slice(&self.buffer[field_offset..]);
            }
            result
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

    decode_le_int!(i16, read_i16);
    decode_le_int!(u16, read_u16);
    decode_le_int!(i32, read_i32);
    decode_le_int!(u32, read_u32);
    decode_le_int!(i64, read_i64);
    decode_le_int!(u64, read_u64);

    pub fn read_address(&mut self, field_offset: usize) -> Address {
        let mut result = Address::default();
        result
            .as_mut_slice()
            .copy_from_slice(&self.buffer[field_offset..]);
        result
    }

    decode_le_bigint!(U160, read_u160);
    decode_le_bigint!(U256, read_u256);

    pub fn read_bytes(&mut self, field_offset: usize) -> &[u8] {
        let bytes_offset = self.read_u32(field_offset + 0) as usize;
        let bytes_length = self.read_u32(field_offset + 4) as usize;
        &self.buffer[bytes_offset..(bytes_offset + bytes_length)]
    }

    pub fn read_bytes2(&mut self, field1_offset: usize, field2_offset: usize) -> (&[u8], &[u8]) {
        let bytes1_offset = self.read_u32(field1_offset + 0) as usize;
        let bytes1_length = self.read_u32(field1_offset + 4) as usize;
        let bytes2_offset = self.read_u32(field2_offset + 0) as usize;
        let bytes2_length = self.read_u32(field2_offset + 4) as usize;
        (
            &self.buffer[bytes1_offset..(bytes1_offset + bytes1_length)],
            &self.buffer[bytes2_offset..(bytes2_offset + bytes2_length)],
        )
    }
}

#[cfg(test)]
mod test {
    use crate::buffer::{BufferDecoder, BufferEncoder};

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
            offset += buffer.write_u64(offset, test.c);
            buffer.finalize()
        };
        println!("{}", hex::encode(&buffer));
        let mut decoder = BufferDecoder::new(buffer.as_slice());
        assert_eq!(decoder.read_u32(0), 100);
        assert_eq!(decoder.read_u16(4), 20);
        assert_eq!(decoder.read_u64(6), 3);
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
