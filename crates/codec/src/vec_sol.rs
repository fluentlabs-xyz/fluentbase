use core::mem::MaybeUninit;

// Global depth counter for WASM (single-threaded environment)
// This is safe in WASM as there's no multi-threading
static mut ENCODING_DEPTH: usize = 0;

/// RAII guard for tracking encoding depth
/// Automatically increments depth on creation and decrements on drop
struct DepthGuard;

impl DepthGuard {
    #[inline]
    fn new() -> (Self, bool) {
        unsafe {
            let is_top_level = ENCODING_DEPTH == 0;
            ENCODING_DEPTH += 1;
            (Self, is_top_level)
        }
    }
}

impl Drop for DepthGuard {
    #[inline]
    fn drop(&mut self) {
        unsafe {
            ENCODING_DEPTH -= 1;
        }
    }
}

/// Solidity ABI encoder implementation for Vec<T>
///
/// # Encoding Rules:
/// - Static types (Vec<u32>): Single pass encoding
/// - Dynamic types (Vec<Vec<T>>): Two-pass encoding
///
/// # Memory Layout:
/// ```text
/// [offset: 32 bytes] -> points to array data (only for top-level)
/// [length: 32 bytes] -> number of elements
/// [offsets: n * 32 bytes] -> only for dynamic elements
/// [data: variable] -> actual element data
/// ```
impl<T, B: ByteOrder, const ALIGN: usize> Encoder<B, ALIGN, true, false> for Vec<T>
where
    T: Default + Sized + Encoder<B, ALIGN, true, false> + Debug,
{
    const HEADER_SIZE: usize = 32;
    const IS_DYNAMIC: bool = true;

    fn encode(&self, buf: &mut impl BufMut, offset: usize) -> Result<usize, CodecError> {
        // Use DepthGuard to determine if this is a top-level or nested encoding
        let (_guard, is_top_level) = DepthGuard::new();

        // Handle offset if provided (for writing at specific position in buffer)
        if offset > 0 {
            if buf.remaining_mut() < offset {
                return Err(CodecError::Encoding(
                    crate::error::EncodingError::InsufficientBuffer {
                        required: offset,
                        available: buf.remaining_mut(),
                    }
                ));
            }
            // Skip to the specified offset
            buf.put_bytes(0, offset);
        }

        // Remember starting position to calculate bytes written
        let start_remaining = buf.remaining_mut();

        // Encode based on element type
        if T::IS_DYNAMIC {
            encode_dynamic_two_pass::<T, B, ALIGN>(self, buf, is_top_level)?;
        } else {
            encode_static_single_pass::<T, B, ALIGN>(self, buf, is_top_level)?;
        }

        // Return number of bytes written
        Ok(start_remaining - buf.remaining_mut())
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        // Use DepthGuard to determine context
        let (_guard, is_top_level) = DepthGuard::new();

        // Read offset to array data
        let data_offset = if is_top_level {
            // Top-level: read pointer from the specified offset
            read_u32_aligned::<B, ALIGN>(buf, offset)?
        } else {
            // Nested: offset directly points to array data
            offset as u32
        } as usize;

        // Read array length
        let length = read_u32_aligned::<B, ALIGN>(buf, data_offset)? as usize;

        if length == 0 {
            return Ok(Vec::new());
        }

        let mut result = Vec::with_capacity(length);

        if T::IS_DYNAMIC {
            // Dynamic elements: read via offset pointers
            let offsets_start = data_offset + 32; // After length field

            for i in 0..length {
                let offset_position = offsets_start + i * 32;
                let element_offset = read_u32_aligned::<B, ALIGN>(buf, offset_position)? as usize;
                // Element offset is relative to the start of offset zone
                let absolute_offset = offsets_start + element_offset;
                result.push(T::decode(buf, absolute_offset)?);
            }
        } else {
            // Static elements: read sequentially
            let mut current_offset = data_offset + 32; // After length field
            let element_size = align_up::<ALIGN>(T::HEADER_SIZE);

            for _ in 0..length {
                result.push(T::decode(buf, current_offset)?);
                current_offset += element_size;
            }
        }

        Ok(result)
    }

    fn partial_decode(buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        // Use DepthGuard for consistency
        let (_guard, is_top_level) = DepthGuard::new();

        let data_offset = if is_top_level {
            read_u32_aligned::<B, ALIGN>(buf, offset)?
        } else {
            offset as u32
        } as usize;

        let length = read_u32_aligned::<B, ALIGN>(buf, data_offset)? as usize;

        // Conservative size estimate
        let element_size = if T::IS_DYNAMIC {
            32
        } else {
            align_up::<ALIGN>(T::HEADER_SIZE)
        };
        let total_size = 32 + 32 + length * element_size;

        Ok((offset, total_size))
    }
}

// Stack-allocated buffer for small arrays to avoid heap allocation
const STACK_BUFFER_SIZE: usize = 16;

/// Single-pass encoding for static element types
///
/// # Example: Vec<u32> = [1, 2, 3]
/// ```text
/// Top-level:
/// Position  | Value | Description
/// ----------|-------|-------------
/// 0x00      | 32    | offset to data (only for top-level)
/// 0x20      | 3     | array length
/// 0x40      | 1     | element[0]
/// 0x60      | 2     | element[1]
/// 0x80      | 3     | element[2]
///
/// Nested:
/// 0x00      | 3     | array length (no offset!)
/// 0x20      | 1     | element[0]
/// 0x40      | 2     | element[1]
/// 0x60      | 3     | element[2]
/// ```
#[inline]
fn encode_static_single_pass<T, B: ByteOrder, const ALIGN: usize>(
    vec: &[T],
    buf: &mut impl BufMut,
    is_top_level: bool,
) -> Result<usize, CodecError>
where
    T: Encoder<B, ALIGN, true, false>,
{
    let mut total_size = 0;

    // Write offset only for top-level arrays
    if is_top_level {
        write_u32_aligned::<B, ALIGN>(buf, 32);
        total_size += 32;
    }

    // Write length
    write_u32_aligned::<B, ALIGN>(buf, vec.len() as u32);
    total_size += 32;

    // Write elements sequentially
    // Elements are written at offset 0 since we write sequentially
    for element in vec.iter() {
        total_size += element.encode(buf, 0)?;
    }

    Ok(total_size)
}

/// Two-pass encoding for dynamic element types
///
/// # Example: Vec<Vec<u32>> = [[1, 2, 3], [4, 5]]
/// ```text
/// PASS 1: Calculate sizes
/// - vec[0] size = 32 (length) + 3*32 (elements) = 128 (no offset for nested!)
/// - vec[1] size = 32 (length) + 2*32 (elements) = 96 (no offset for nested!)
///
/// PASS 2: Write with calculated offsets
/// Position  | Value | Description
/// ----------|-------|-------------
/// 0x00      | 32    | offset to array data (only for top-level)
/// 0x20      | 2     | array length
/// 0x40      | 64    | offset[0] (relative to 0x40)
/// 0x60      | 192   | offset[1] (relative to 0x40)
/// 0x80      | ...   | data for vec[0]
/// 0x100     | ...   | data for vec[1]
/// ```
fn encode_dynamic_two_pass<T, B: ByteOrder, const ALIGN: usize>(
    vec: &[T],
    buf: &mut impl BufMut,
    is_top_level: bool,
) -> Result<usize, CodecError>
where
    T: Encoder<B, ALIGN, true, false>,
{
    // Optimization: use stack buffer for small arrays
    if vec.len() <= STACK_BUFFER_SIZE {
        encode_dynamic_stack_optimized::<T, B, ALIGN>(vec, buf, is_top_level)
    } else {
        encode_dynamic_heap::<T, B, ALIGN>(vec, buf, is_top_level)
    }
}

/// Stack-optimized version for small arrays (no heap allocation)
#[inline]
fn encode_dynamic_stack_optimized<T, B: ByteOrder, const ALIGN: usize>(
    vec: &[T],
    buf: &mut impl BufMut,
    is_top_level: bool,
) -> Result<usize, CodecError>
where
    T: Encoder<B, ALIGN, true, false>,
{
    // Use simple array of usize instead of SizeInfo struct
    let mut sizes = [0usize; STACK_BUFFER_SIZE];

    // PASS 1: Calculate sizes
    for (i, element) in vec.iter().enumerate() {
        sizes[i] = calculate_encoded_size::<T, B, ALIGN>(element)?;
    }

    // Write using calculated sizes
    write_with_sizes::<T, B, ALIGN>(vec, buf, &sizes[..vec.len()], is_top_level)
}

/// Heap version for large arrays
#[inline]
fn encode_dynamic_heap<T, B: ByteOrder, const ALIGN: usize>(
    vec: &[T],
    buf: &mut impl BufMut,
    is_top_level: bool,
) -> Result<usize, CodecError>
where
    T: Encoder<B, ALIGN, true, false>,
{
    // PASS 1: Calculate sizes of all elements
    let sizes: Vec<usize> = vec
        .iter()
        .map(|element| calculate_encoded_size::<T, B, ALIGN>(element))
        .collect::<Result<Vec<_>, CodecError>>()?;

    // Write using calculated sizes
    write_with_sizes::<T, B, ALIGN>(vec, buf, &sizes, is_top_level)
}

/// Common write logic using pre-calculated sizes
#[inline(always)]
fn write_with_sizes<T, B: ByteOrder, const ALIGN: usize>(
    vec: &[T],
    buf: &mut impl BufMut,
    sizes: &[usize],
    is_top_level: bool,
) -> Result<usize, CodecError>
where
    T: Encoder<B, ALIGN, true, false>,
{
    let mut total_size = 0;

    // Write header offset only for top-level arrays
    if is_top_level {
        write_u32_aligned::<B, ALIGN>(buf, 32);
        total_size += 32;
    }

    // Write array length
    write_u32_aligned::<B, ALIGN>(buf, vec.len() as u32);
    total_size += 32;

    // Calculate and write element offsets for dynamic arrays
    let offsets_size = vec.len() * 32;
    total_size += offsets_size;

    // Element offsets are relative to the start of the offset zone
    let mut element_offset = offsets_size; // First element starts after all offsets

    for &size in sizes.iter() {
        write_u32_aligned::<B, ALIGN>(buf, element_offset as u32);
        element_offset += size;
    }

    // PASS 2: Write actual element data
    // Elements are written sequentially, so offset is always 0
    for element in vec.iter() {
        element.encode(buf, 0)?;
    }

    // Add sizes of all elements
    total_size += sizes.iter().sum::<usize>();

    Ok(total_size)
}

/// Calculate encoded size without actually encoding
/// Uses a counting buffer to avoid memory allocation
#[inline]
fn calculate_encoded_size<T, B: ByteOrder, const ALIGN: usize>(
    element: &T,
) -> Result<usize, CodecError>
where
    T: Encoder<B, ALIGN, true, false>,
{
    let mut counter = ByteCounter::new();
    // Encode with offset 0 since ByteCounter doesn't write actual data
    element.encode(&mut counter, 0)?;
    Ok(counter.count())
}

/// A BufMut implementation that only counts bytes without storing them
struct ByteCounter {
    count: usize,
}

impl ByteCounter {
    #[inline(always)]
    fn new() -> Self {
        Self { count: 0 }
    }

    #[inline(always)]
    fn count(&self) -> usize {
        self.count
    }
}

unsafe impl BufMut for ByteCounter {
    #[inline(always)]
    fn remaining_mut(&self) -> usize {
        usize::MAX
    }

    #[inline(always)]
    unsafe fn advance_mut(&mut self, cnt: usize) {
        self.count += cnt;
    }

    // Explicitly forbid chunk_mut usage
    #[inline(always)]
    fn chunk_mut(&mut self) -> &mut bytes::buf::UninitSlice {
        panic!("chunk_mut should not be called on ByteCounter");
    }

    // Optimize common write operations to just count bytes
    #[inline(always)]
    fn put_u32(&mut self, _: u32) {
        self.count += 4;
    }

    #[inline(always)]
    fn put_slice(&mut self, src: &[u8]) {
        self.count += src.len();
    }

    #[inline(always)]
    fn put_u8(&mut self, _: u8) {
        self.count += 1;
    }

    #[inline(always)]
    fn put_u16(&mut self, _: u16) {
        self.count += 2;
    }

    #[inline(always)]
    fn put_u64(&mut self, _: u64) {
        self.count += 8;
    }
}

// ============================================================================
// DETAILED EXAMPLE: How Vec<Vec<Vec<u32>>> = [[[1,2], [3]], [[4,5,6]]] works
// ============================================================================
//
// With DepthGuard tracking:
//
// LEVEL 0 (outer vector):
// ----------------------
// ENCODING_DEPTH: 0 -> 1 (is_top_level = true)
// PASS 1:
//   - element[0]: ENCODING_DEPTH: 1 -> 2 (is_top_level = false)
//   - element[1]: ENCODING_DEPTH: 1 -> 2 (is_top_level = false)
//
// PASS 2:
//   0x000: write offset = 32 (only because top-level)
//   0x020: write length = 2
//   0x040: write offset[0] = 64 (relative to 0x40)
//   0x060: write offset[1] = 288 (relative to 0x40)
//   0x080: write element[0] data (nested encoding)
//   0x180: write element[1] data (nested encoding)
//
// LEVEL 1 (middle vectors):
// ------------------------
// For element[0] = [[1,2], [3]]:
// ENCODING_DEPTH: 1 -> 2 (is_top_level = false)
//   - NO offset header!
//   - Writes length and offsets for inner arrays
//
// LEVEL 2 (inner vectors):
// -----------------------
// For [1,2]:
// ENCODING_DEPTH: 2 -> 3 (is_top_level = false)
//   - NO offset header!
//   - Single pass (static elements)
//   - Writes: length(32) + 1(32) + 2(32) = 96 bytes
