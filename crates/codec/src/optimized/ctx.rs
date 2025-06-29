use smallvec::SmallVec;

#[derive(Debug, PartialOrd, PartialEq, Clone)]
#[repr(C)]
pub struct EncodingContext {
    pub hdr_size: u32,
    pub hdr_ptr: u32,
    pub data_ptr: u32,
    pub depth: u8,

}

impl Default for EncodingContext {
    fn default() -> Self {
        Self {
            hdr_size: 0,
            hdr_ptr: 0,
            data_ptr: 0,
            depth: 0,
        }
    }
}

impl EncodingContext {
    pub fn new() -> Self {
        Self::default()
    }

   
}

