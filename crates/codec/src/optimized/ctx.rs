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
#[derive(Default)]
pub struct EncodingContext {
    pub hdr_size: u32,
    pub hdr_ptr: u32,
    pub data_ptr: u32,
    pub depth: u32,
    pub header_encoded: bool,
}


impl EncodingContext {
    pub fn new() -> Self {
        Self::default()
    }
}