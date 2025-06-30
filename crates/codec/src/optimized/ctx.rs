use smallvec::SmallVec;

#[derive(Debug, PartialOrd, PartialEq, Clone)]
#[repr(C)]
pub struct StackFrame {
    pub hdr_size: u32,
    pub hdr_ptr: u32,
    pub data_ptr: u32,
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
#[repr(C)]
pub struct EncodingContext {
    pub hdr_size: u32,
    pub hdr_ptr: u32,
    pub data_ptr: u32,
    pub depth: u32,
    pub header_encoded: bool,
}

impl Default for EncodingContext {
    fn default() -> Self {
        Self {
            hdr_size: 0,
            hdr_ptr: 0,
            data_ptr: 0,
            depth: 0,
            header_encoded: false,
        }
    }
}

impl EncodingContext {
    pub fn new() -> Self {
        Self::default()
    }
}