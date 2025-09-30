/// Interprets a byte slice as a u32 slice if length is a multiple of 4; otherwise returns None.
///
/// # Safety
/// This function is safe because:
/// 1. We check that the slice length is a multiple of 4
/// 2. We ensure proper alignment by checking the pointer alignment
/// 3. The resulting slice has the correct length (slice.len() / 4)
pub fn cast_u8_to_u32(slice: &[u8]) -> Option<&[u32]> {
    if slice.len() % 4 != 0 {
        return None;
    }

    // Additional safety check: ensure the pointer is properly aligned
    if slice.as_ptr() as usize % 4 != 0 {
        return None;
    }

    // Safe because we've verified alignment and length
    Some(unsafe { std::slice::from_raw_parts(slice.as_ptr() as *const u32, slice.len() / 4) })
}
