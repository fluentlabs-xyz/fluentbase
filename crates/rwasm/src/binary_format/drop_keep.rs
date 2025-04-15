use crate::binary_format::{
    BinaryFormat,
    BinaryFormatError,
    BinaryFormatReader,
    BinaryFormatWriter,
};
use rwasm::engine::DropKeep;

impl<'a> BinaryFormat<'a> for DropKeep {
    type SelfType = DropKeep;

    fn encoded_length(&self) -> usize {
        8
    }

    fn write_binary(&self, sink: &mut BinaryFormatWriter<'a>) -> Result<usize, BinaryFormatError> {
        let mut n = 0;
        n += sink.write_u32_le(self.drop() as u32)?;
        n += sink.write_u32_le(self.keep() as u32)?;
        Ok(n)
    }

    fn read_binary(sink: &mut BinaryFormatReader<'a>) -> Result<Self::SelfType, BinaryFormatError> {
        let drop = sink.read_u32_le()?;
        let keep = sink.read_u32_le()?;
        Ok(DropKeep::new(drop as usize, keep as usize).unwrap())
    }
}
