use smallvec::SmallVec;

///
#[derive(Debug, PartialOrd, PartialEq)]
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

impl Default for EncodingContext {
    fn default() -> Self {
        let mut meta_base = SmallVec::<[u32; 16]>::new();
        meta_base.push(0);
        Self {
            hdr_size: 0,
            hdr_ptr: 0,
            data_ptr: 0,
        }
    }
}
impl EncodingContext {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }
}
