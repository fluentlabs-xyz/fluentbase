use crate::compute_budget::compute_budget::HEAP_LENGTH;

pub const MIN_HEAP_FRAME_BYTES: u32 = HEAP_LENGTH as u32 * 2i32.pow(0) as u32;
pub const MAX_HEAP_FRAME_BYTES: u32 = HEAP_LENGTH as u32 * 2i32.pow(8) as u32;
