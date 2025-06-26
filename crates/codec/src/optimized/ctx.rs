use crate::optimized::{
    error::CodecError,
    utils::{align_up, write_u32_aligned},
};
use byteorder::ByteOrder;
use bytes::BufMut;
use smallvec::SmallVec;

/// One vector-node: `len` = children, `tail` = data bytes (u32 fits <4 GB)
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialOrd, PartialEq)]
pub struct NodeMeta {
    pub len:     u32,   // number of children
    pub tail:    u32,   // bytes of raw body
    pub total_hdr_len: u32,   // bytes of headers in subtree (including self)
}

#[derive(Default, Debug, PartialOrd, PartialEq)]
#[repr(C)]
pub struct EncodingContext {
    /// DFS-flattened list filled in `build_ctx`
    pub nodes: SmallVec<[NodeMeta; 32]>,
    /// next NodeMeta to encode
    pub index: usize,
    pub hdr_written:  u32,  // bytes of headers already written in the whole buffer
    pub body_reserved: u32, // bytes of body already reserved in the whole buffer
    pub root_hdr:     u32, // total header size (used in structs) all headers+all statics

}

impl EncodingContext {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }


}
