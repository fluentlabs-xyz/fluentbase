use alloc::vec::Vec;
use bincode::enc::Encoder;
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

pub struct VecEncoder<'a, T>(pub &'a mut Vec<T>);

impl<'a, T> VecEncoder<'a, T> {
    pub fn new(buf: &mut Vec<T>) -> VecEncoder<'_, T> {
        VecEncoder(buf)
    }
}
// impl<'a> Encoder for VecEncoder<'a, Vec<u8>> {
//     type W = VecWriter<'a, u8>;
//     type C = bincode::config::Configuration;
//
//     fn writer(&mut self) -> &mut Self::W {
//         todo!()
//     }
//
//     fn config(&self) -> &Self::C {
//         todo!()
//     }
// }
