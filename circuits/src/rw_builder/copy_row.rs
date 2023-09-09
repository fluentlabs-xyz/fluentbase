use crate::impl_expr;
use std::{fmt, fmt::Formatter};
use strum_macros::EnumIter;

#[derive(Debug, Clone, Copy, EnumIter)]
pub enum CopyTableTag {
    // copy from input to memory (_sys_read)
    Input = 1,
    // copy from memory to output (_sys_write)
    Output,
}

impl_expr!(CopyTableTag);

impl fmt::Display for CopyTableTag {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            CopyTableTag::Input => write!(f, "Input"),
            CopyTableTag::Output => write!(f, "Output"),
        }
    }
}

impl Into<usize> for CopyTableTag {
    fn into(self) -> usize {
        self as usize
    }
}

pub const N_COPY_TABLE_TAG_BITS: usize = 2;

#[derive(Debug, Clone)]
pub struct CopyRow {
    pub tag: CopyTableTag,
    pub from_address: u32,
    pub to_address: u32,
    pub length: u32,
    pub rw_counter: usize,
    pub data: Vec<u8>,
}
