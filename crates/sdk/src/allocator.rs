use alloc::vec::Vec;

const WASM_PAGE_SIZE_IN_BYTES: usize = 65536;

#[allow(dead_code)]
fn calc_pages_needed(pages_allocated: usize, ptr: usize) -> usize {
    let current_memory = pages_allocated * WASM_PAGE_SIZE_IN_BYTES;
    if ptr >= current_memory {
        (current_memory + ptr + 1 + WASM_PAGE_SIZE_IN_BYTES - 1) / WASM_PAGE_SIZE_IN_BYTES
            - pages_allocated
    } else {
        0
    }
}

#[test]
fn test_pages_needed() {
    assert_eq!(calc_pages_needed(0, 1), 1);
    assert_eq!(calc_pages_needed(0, 65535), 1);
    assert_eq!(calc_pages_needed(0, 65536), 2);
    assert_eq!(calc_pages_needed(1, 65536), 2);
    assert_eq!(calc_pages_needed(1, 65535), 0);
    assert_eq!(calc_pages_needed(1, 65536 * 2), 3);
    assert_eq!(calc_pages_needed(5, 327680), 6);
}

#[inline(always)]
pub fn alloc_ptr(len: usize) -> *mut u8 {
    unsafe { alloc::alloc::alloc(core::alloc::Layout::from_size_align_unchecked(len, 8)) }
}

#[inline(always)]
pub fn alloc_slice<'a>(len: usize) -> &'a mut [u8] {
    use core::ptr;
    unsafe { &mut *ptr::slice_from_raw_parts_mut(alloc_ptr(len), len) }
}

#[inline(always)]
pub fn alloc_vec(len: usize) -> Vec<u8> {
    alloc_slice(len).to_vec()
}

#[cfg(target_arch = "wasm32")]
pub struct HeapBaseAllocator {}

#[cfg(target_arch = "wasm32")]
static mut HEAP_CHECKPOINT_IDX: usize = 0;

#[cfg(target_arch = "wasm32")]
static mut ALLOC_COUNT: usize = 0;

#[cfg(target_arch = "wasm32")]
static mut ALLOC_BYTES: usize = 0;

#[cfg(target_arch = "wasm32")]
static mut DEALLOC_TRY_COUNT: usize = 0;

#[cfg(target_arch = "wasm32")]
static mut DEALLOC_TRY_BYTES: usize = 0;

#[cfg(target_arch = "wasm32")]
static mut DEALLOC_COUNT: usize = 0;

#[cfg(target_arch = "wasm32")]
static mut DEALLOC_BYTES: usize = 0;

#[cfg(target_arch = "wasm32")]
static ENABLE_SIMPLE_DEALLOC: bool = false;

#[cfg(target_arch = "wasm32")]
static mut HEAP_CHECKPOINTS: [usize; 1024] = [0usize; 1024];

#[cfg(target_arch = "wasm32")]
static HEAP_FILL_WITH_0_ON_RESET: bool = true;

#[cfg(target_arch = "wasm32")]
static mut HEAP_POS_PREV_IDX: usize = 0;

#[cfg(target_arch = "wasm32")]
static mut HEAP_POS_PREV_CHECKPOINTS: [usize; 1024 * 8] = [0usize; 1024 * 8];

#[cfg(target_arch = "wasm32")]
static mut HEAP_POS: usize = 0;

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub fn __heap_checkpoint_idx() -> usize {
    unsafe { HEAP_CHECKPOINT_IDX }
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub fn __dealloc_try_count() -> usize {
    unsafe { DEALLOC_TRY_COUNT }
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub fn __dealloc_try_bytes() -> usize {
    unsafe { DEALLOC_TRY_BYTES }
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub fn __alloc_count() -> usize {
    unsafe { ALLOC_COUNT }
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub fn __alloc_bytes() -> usize {
    unsafe { ALLOC_BYTES }
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub fn __dealloc_count() -> usize {
    unsafe { DEALLOC_COUNT }
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub fn __dealloc_bytes() -> usize {
    unsafe { DEALLOC_BYTES }
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub fn __heap_checkpoint_save() {
    unsafe {
        HEAP_CHECKPOINTS[HEAP_CHECKPOINT_IDX] = HEAP_POS;
        HEAP_CHECKPOINT_IDX += 1;
    }
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub fn __heap_checkpoint_pop() -> usize {
    unsafe {
        HEAP_CHECKPOINT_IDX -= 1;
        let heap_pos = HEAP_CHECKPOINTS[HEAP_CHECKPOINT_IDX];
        HEAP_POS = heap_pos;
        heap_pos
    }
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub fn __heap_checkpoint_peek_last() -> usize {
    unsafe { HEAP_CHECKPOINTS[HEAP_CHECKPOINT_IDX - 1] }
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub fn __heap_pos() -> usize {
    unsafe { HEAP_POS }
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub fn __heap_pos_set(value: usize) {
    unsafe { HEAP_POS = value };
}

#[cfg(target_arch = "wasm32")]
unsafe impl core::alloc::GlobalAlloc for HeapBaseAllocator {
    #[inline(never)]
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let bytes: usize = layout.size();
        let align: usize = layout.align();
        unsafe {
            ALLOC_COUNT += 1;
            ALLOC_BYTES += bytes;
        }
        extern "C" {
            static __heap_base: u8;
        }
        let mut heap_pos = unsafe { HEAP_POS };
        if heap_pos == 0 {
            heap_pos = unsafe { (&__heap_base) as *const u8 as usize };
        }
        let offset = heap_pos & (align - 1);
        if offset != 0 {
            heap_pos += align - offset;
        }
        // allocate memory pages if needed
        let pages_allocated = core::arch::wasm32::memory_size::<0>();
        let pages_needed = calc_pages_needed(pages_allocated, heap_pos + bytes);
        if pages_needed > 0 {
            let new_pages = core::arch::wasm32::memory_grow::<0>(pages_needed);
            if new_pages == usize::MAX {
                // TODO(dmitry123): "how to use trap code here?"
                unsafe {
                    core::hint::unreachable_unchecked();
                }
            }
        }
        // return allocated pointer
        let ptr = heap_pos as *mut u8;
        heap_pos += bytes;
        unsafe {
            HEAP_POS_PREV_CHECKPOINTS[HEAP_POS_PREV_IDX] = HEAP_POS;
            HEAP_POS_PREV_IDX += 1;
            HEAP_POS = heap_pos;
        };
        ptr
    }

    #[inline(always)]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        // ops, no dealoc
        unsafe {
            DEALLOC_TRY_COUNT += 1;
            DEALLOC_TRY_BYTES += layout.size();
            let dealloc_chunk_base_ptr = ptr as usize;
            if ENABLE_SIMPLE_DEALLOC
                && HEAP_POS_PREV_IDX > 0
                && dealloc_chunk_base_ptr >= HEAP_POS_PREV_CHECKPOINTS[HEAP_POS_PREV_IDX - 1]
            {
                DEALLOC_COUNT += 1;
                DEALLOC_BYTES += layout.size();
                HEAP_POS_PREV_IDX -= 1;
                __heap_pos_set(HEAP_POS_PREV_CHECKPOINTS[HEAP_POS_PREV_IDX]);
            }
        };
    }
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn __heap_reset() -> usize {
    extern "C" {
        static mut __heap_base: u8;
    }
    let heap_pos_current = unsafe { HEAP_POS };
    let heap_pos_base = unsafe { (&__heap_base) as *const u8 as usize };
    if HEAP_FILL_WITH_0_ON_RESET {
        unsafe {
            core::slice::from_raw_parts_mut(
                (&mut __heap_base) as *mut u8,
                heap_pos_current - heap_pos_base,
            )
            .fill(0);
        }
    }
    unsafe {
        HEAP_POS = (&__heap_base) as *const u8 as usize;
        HEAP_POS
    }
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn __heap_base_offset() -> usize {
    extern "C" {
        static __heap_base: u8;
    }
    unsafe { (&__heap_base) as *const u8 as usize }
}
