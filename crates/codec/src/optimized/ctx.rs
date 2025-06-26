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
    pub len: u32,
    pub tail: u32,
}

/// Minimal encoder context (32-bit friendly)
#[derive(Default, Debug, PartialOrd, PartialEq)]
#[repr(C)]
pub struct EncodingContext {
    pub depth: u8, // current recursion level
    // 2 bytes implicit padding inserted by the compiler -> 4-alignment ok
    pub nodes: SmallVec<[NodeMeta; 32]>,
}

impl EncodingContext {
    #[inline]
    pub fn new() -> Self {
        Self {
            depth: 0,
            nodes: SmallVec::new(),
        }
    }

    /* depth guard --------------------------------------------------- */
    #[inline]
    pub fn enter(&mut self) -> Result<(), CodecError> {
        if self.depth >= 32 {
            return Err(CodecError::InvalidData("max depth exceeded"));
        }
        self.depth += 1;
        Ok(())
    }
    #[inline]
    pub fn exit(&mut self) {
        self.depth -= 1;
    }

    #[inline]
    pub fn reset(&mut self) {
        self.depth = 0;
        self.nodes.clear();
    }
}
