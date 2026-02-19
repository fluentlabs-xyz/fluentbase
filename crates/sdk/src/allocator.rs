#[cfg(target_arch = "wasm32")]
mod block_list;
#[cfg(target_arch = "wasm32")]
mod heap_base;

#[cfg(target_arch = "wasm32")]
pub use self::{block_list::BlockListAllocator, heap_base::HeapBaseAllocator};

#[inline(always)]
pub fn alloc_ptr_unaligned(len: usize) -> *mut u8 {
    if len == 0 {
        return core::ptr::null_mut();
    }
    let layout = core::alloc::Layout::from_size_align(len, 1).unwrap();
    unsafe { alloc::alloc::alloc(layout) }
}

#[allow(dead_code)]
fn calc_pages_needed(pages_allocated: usize, required_bytes: usize) -> usize {
    const WASM_PAGE_SIZE_IN_BYTES: usize = 65536;
    let have = pages_allocated * WASM_PAGE_SIZE_IN_BYTES;
    if required_bytes <= have {
        return 0;
    }
    let missing = required_bytes - have;
    (missing + WASM_PAGE_SIZE_IN_BYTES - 1) / WASM_PAGE_SIZE_IN_BYTES
}

#[test]
fn test_pages_needed() {
    assert_eq!(calc_pages_needed(0, 1), 1);
    assert_eq!(calc_pages_needed(0, 65535), 1);
    assert_eq!(calc_pages_needed(0, 65536), 1);
    assert_eq!(calc_pages_needed(1, 65536), 0);
    assert_eq!(calc_pages_needed(1, 65535), 0);
    assert_eq!(calc_pages_needed(1, 65536 + 65536), 1);
    assert_eq!(calc_pages_needed(5, 327680), 0);
}
