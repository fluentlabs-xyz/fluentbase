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

/// Alternative function that handles unaligned byte slices by copying to aligned storage
pub fn cast_u8_to_u32_aligned(slice: &[u8]) -> Option<Vec<u32>> {
    if slice.len() % 4 != 0 {
        return None;
    }

    // Convert bytes to u32 words, handling endianness
    let mut words = Vec::with_capacity(slice.len() / 4);
    for chunk in slice.chunks_exact(4) {
        let word = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        words.push(word);
    }

    Some(words)
}
