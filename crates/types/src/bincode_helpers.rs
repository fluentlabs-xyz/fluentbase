use alloc::vec::Vec;
use bincode::error::EncodeError;

pub struct VecWriter<'a, T>(pub &'a mut Vec<T>);
impl<T> VecWriter<'_, T> {
    pub fn new(buf: &mut Vec<T>) -> VecWriter<'_, T> {
        VecWriter(buf)
    }
}
impl<'a> bincode::enc::write::Writer for VecWriter<'a, u8> {
    fn write(&mut self, bytes: &[u8]) -> Result<(), EncodeError> {
        self.0.extend_from_slice(bytes);
        Ok(())
    }
}
