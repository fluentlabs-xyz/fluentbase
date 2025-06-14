use alloc::vec::Vec;
use byteorder::ByteOrder;
use solana_instruction::error::InstructionError;

pub struct VecU8 {
    pub vec: Vec<u8>,
}

impl VecU8 {
    pub fn new(vec: Vec<u8>) -> Self {
        Self { vec }
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.vec.len()
    }
    #[inline]
    pub fn write_all(&mut self, buf: &[u8]) -> Result<(), InstructionError> {
        self.vec.extend_from_slice(buf);
        Ok(())
    }

    #[inline]
    pub fn write_u8(&mut self, n: u8) -> Result<(), InstructionError> {
        self.write_all(&[n])
    }

    #[inline]
    pub fn write_u64<V: ByteOrder>(&mut self, n: u64) -> Result<(), InstructionError> {
        let mut buf = [0; 8];
        V::write_u64(&mut buf, n);
        self.write_all(&buf)
    }

    #[inline]
    pub fn extend_from_slice<V: ByteOrder>(&mut self, other: &[u8]) {
        Vec::extend_from_slice(&mut self.vec, other);
    }
}
