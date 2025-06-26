use crate::optimized::encoder::Encoder;
use byteorder::BigEndian;

///
#[derive(Default, Debug, PartialOrd, PartialEq)]
#[repr(C)]
pub struct EncodingContext {
    /// the full size of the header section
    /// for structs and tuples, this is the sum of all headers and all static fields
    pub hdr_size: u32,

    /// this fields advances only inside actual encoding,
    /// tail of the header section
    pub hdr_ptr: u32,
    /// tail of the body section
    pub data_ptr: u32,
}

impl EncodingContext {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }
}

