use crate::binary_format::BinaryFormatError;
use alloc::vec::Vec;
use byteorder::{BigEndian, ByteOrder, LittleEndian};

pub struct BinaryFormatWriter<'a> {
    pub sink: &'a mut [u8],
    pub pos: usize,
}

impl<'a> BinaryFormatWriter<'a> {
    pub fn new(sink: &'a mut [u8]) -> Self {
        Self { sink, pos: 0 }
    }

    pub fn write_u8(&mut self, value: u8) -> Result<usize, BinaryFormatError> {
        let n = self.require(1)?;
        self.sink[self.pos] = value;
        self.skip(n)
    }

    pub fn write_u16_be(&mut self, value: u16) -> Result<usize, BinaryFormatError> {
        let n = self.require(2)?;
        BigEndian::write_u16(&mut self.sink[self.pos..], value);
        self.skip(n)
    }

    pub fn write_u16_le(&mut self, value: u16) -> Result<usize, BinaryFormatError> {
        let n = self.require(2)?;
        LittleEndian::write_u16(&mut self.sink[self.pos..], value);
        self.skip(n)
    }

    pub fn write_i16_be(&mut self, value: i16) -> Result<usize, BinaryFormatError> {
        let n = self.require(2)?;
        BigEndian::write_i16(&mut self.sink[self.pos..], value);
        self.skip(n)
    }

    pub fn write_i16_le(&mut self, value: i16) -> Result<usize, BinaryFormatError> {
        let n = self.require(2)?;
        LittleEndian::write_i16(&mut self.sink[self.pos..], value);
        self.skip(n)
    }

    pub fn write_u32_be(&mut self, value: u32) -> Result<usize, BinaryFormatError> {
        let n = self.require(4)?;
        BigEndian::write_u32(&mut self.sink[self.pos..], value);
        self.skip(n)
    }

    pub fn write_u32_le(&mut self, value: u32) -> Result<usize, BinaryFormatError> {
        let n = self.require(4)?;
        LittleEndian::write_u32(&mut self.sink[self.pos..], value);
        self.skip(n)
    }

    pub fn write_i32_be(&mut self, value: i32) -> Result<usize, BinaryFormatError> {
        let n = self.require(4)?;
        BigEndian::write_i32(&mut self.sink[self.pos..], value);
        self.skip(n)
    }

    pub fn write_i32_le(&mut self, value: i32) -> Result<usize, BinaryFormatError> {
        let n = self.require(4)?;
        LittleEndian::write_i32(&mut self.sink[self.pos..], value);
        self.skip(n)
    }

    pub fn write_u64_be(&mut self, value: u64) -> Result<usize, BinaryFormatError> {
        let n = self.require(8)?;
        BigEndian::write_u64(&mut self.sink[self.pos..], value);
        self.skip(n)
    }

    pub fn write_u64_le(&mut self, value: u64) -> Result<usize, BinaryFormatError> {
        let n = self.require(8)?;
        LittleEndian::write_u64(&mut self.sink[self.pos..], value);
        self.skip(n)
    }

    pub fn write_i64_be(&mut self, value: i64) -> Result<usize, BinaryFormatError> {
        let n = self.require(8)?;
        BigEndian::write_i64(&mut self.sink[self.pos..], value);
        self.skip(n)
    }

    pub fn write_i64_le(&mut self, value: i64) -> Result<usize, BinaryFormatError> {
        let n = self.require(8)?;
        LittleEndian::write_i64(&mut self.sink[self.pos..], value);
        self.skip(n)
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<usize, BinaryFormatError> {
        let n = self.require(bytes.len())?;
        self.sink[self.pos..(self.pos + n)].copy_from_slice(bytes);
        self.skip(n)
    }

    fn require(&self, n: usize) -> Result<usize, BinaryFormatError> {
        if self.sink.len() < self.pos + n {
            Err(BinaryFormatError::NeedMore(self.pos + n - self.sink.len()))
        } else {
            Ok(n)
        }
    }

    pub fn reset(&mut self) {
        self.pos = 0;
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    fn skip(&mut self, n: usize) -> Result<usize, BinaryFormatError> {
        assert!(self.sink.len() >= self.pos + n);
        self.pos += n;
        Ok(n)
    }

    pub fn to_vec(&self) -> Vec<u8> {
        self.sink[0..self.pos].to_vec()
    }
}

pub struct BinaryFormatReader<'a> {
    pub sink: &'a [u8],
    pub pos: usize,
}

impl<'a> BinaryFormatReader<'a> {
    pub fn new(sink: &'a [u8]) -> Self {
        Self { sink, pos: 0 }
    }

    pub fn limit_with(&self, length: usize) -> Self {
        Self {
            sink: &self.sink[self.pos..(self.pos + length)],
            pos: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.pos >= self.sink.len()
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn assert_u8(&mut self, value: u8) -> Result<u8, BinaryFormatError> {
        if self.read_u8()? != value {
            Err(BinaryFormatError::MalformedWasmModule)
        } else {
            Ok(1)
        }
    }

    pub fn read_u8(&mut self) -> Result<u8, BinaryFormatError> {
        self.require(1)?;
        let result = self.sink[self.pos];
        self.skip(1)?;
        Ok(result)
    }

    pub fn read_u16_be(&mut self) -> Result<u16, BinaryFormatError> {
        self.require(2)?;
        let result = BigEndian::read_u16(&self.sink[self.pos..]);
        self.skip(2)?;
        Ok(result)
    }

    pub fn read_u16_le(&mut self) -> Result<u16, BinaryFormatError> {
        self.require(2)?;
        let result = LittleEndian::read_u16(&self.sink[self.pos..]);
        self.skip(2)?;
        Ok(result)
    }

    pub fn read_i16_be(&mut self) -> Result<i16, BinaryFormatError> {
        self.require(2)?;
        let result = BigEndian::read_i16(&self.sink[self.pos..]);
        self.skip(2)?;
        Ok(result)
    }

    pub fn read_i16_le(&mut self) -> Result<i16, BinaryFormatError> {
        self.require(2)?;
        let result = LittleEndian::read_i16(&self.sink[self.pos..]);
        self.skip(2)?;
        Ok(result)
    }

    pub fn read_u32_be(&mut self) -> Result<u32, BinaryFormatError> {
        self.require(4)?;
        let result = BigEndian::read_u32(&self.sink[self.pos..]);
        self.skip(4)?;
        Ok(result)
    }

    pub fn read_u32_le(&mut self) -> Result<u32, BinaryFormatError> {
        self.require(4)?;
        let result = LittleEndian::read_u32(&self.sink[self.pos..]);
        self.skip(4)?;
        Ok(result)
    }

    pub fn read_i32_be(&mut self) -> Result<i32, BinaryFormatError> {
        self.require(4)?;
        let result = BigEndian::read_i32(&self.sink[self.pos..]);
        self.skip(4)?;
        Ok(result)
    }

    pub fn read_i32_le(&mut self) -> Result<i32, BinaryFormatError> {
        self.require(4)?;
        let result = LittleEndian::read_i32(&self.sink[self.pos..]);
        self.skip(4)?;
        Ok(result)
    }

    pub fn read_u64_be(&mut self) -> Result<u64, BinaryFormatError> {
        self.require(8)?;
        let result = BigEndian::read_u64(&self.sink[self.pos..]);
        self.skip(8)?;
        Ok(result)
    }

    pub fn read_u64_le(&mut self) -> Result<u64, BinaryFormatError> {
        self.require(8)?;
        let result = LittleEndian::read_u64(&self.sink[self.pos..]);
        self.skip(8)?;
        Ok(result)
    }

    pub fn read_i64_be(&mut self) -> Result<i64, BinaryFormatError> {
        self.require(8)?;
        let result = BigEndian::read_i64(&self.sink[self.pos..]);
        self.skip(8)?;
        Ok(result)
    }

    pub fn read_i64_le(&mut self) -> Result<i64, BinaryFormatError> {
        self.require(8)?;
        let result = LittleEndian::read_i64(&self.sink[self.pos..]);
        self.skip(8)?;
        Ok(result)
    }

    pub fn read_bytes(&mut self, bytes: &mut [u8]) -> Result<(), BinaryFormatError> {
        self.require(bytes.len())?;
        bytes.copy_from_slice(&self.sink[self.pos..(self.pos + bytes.len())]);
        self.skip(bytes.len())
    }

    fn require(&self, n: usize) -> Result<(), BinaryFormatError> {
        if self.sink.len() < self.pos + n {
            Err(BinaryFormatError::NeedMore(self.pos + n - self.sink.len()))
        } else {
            Ok(())
        }
    }

    fn skip(&mut self, n: usize) -> Result<(), BinaryFormatError> {
        assert!(self.sink.len() >= self.pos + n);
        self.pos += n;
        Ok(())
    }
}
