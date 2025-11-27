use alloc::vec::Vec;

const WASM_PAGE_SIZE_IN_BYTES: usize = 65536;

#[allow(dead_code)]
fn calc_pages_needed(pages_allocated: usize, required_bytes: usize) -> usize {
    let have = pages_allocated * WASM_PAGE_SIZE_IN_BYTES;
    if required_bytes <= have {
        0
    } else {
        let missing = required_bytes - have;
        (missing + WASM_PAGE_SIZE_IN_BYTES - 1) / WASM_PAGE_SIZE_IN_BYTES
    }
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

#[inline(always)]
pub fn alloc_ptr(len: usize) -> *mut u8 {
    unsafe { alloc::alloc::alloc_zeroed(core::alloc::Layout::from_size_align_unchecked(len, 8)) }
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
static mut HEAP_POS: usize = 0;

#[inline(always)]
pub fn alloc_heap_pos() -> usize {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        HEAP_POS
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        usize::MAX
    }
}

#[inline(always)]
pub fn rollback_heap_pos(_new_heap_pos: usize) {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        HEAP_POS = _new_heap_pos
    }
}

#[cfg(target_arch = "wasm32")]
unsafe impl core::alloc::GlobalAlloc for HeapBaseAllocator {
    #[inline(never)]
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let bytes: usize = layout.size();
        let align: usize = layout.align();
        extern "C" {
            static __heap_base: u8;
        }
        let mut heap_pos = unsafe { HEAP_POS };
        if heap_pos == 0 {
            heap_pos = unsafe { (&__heap_base) as *const u8 as usize };
        }
        let offset = heap_pos & (align - 1);
        if offset != 0 {
            heap_pos = heap_pos.wrapping_add(align - offset);
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
        unsafe { HEAP_POS = heap_pos };
        ptr
    }

    #[inline(always)]
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {
        // ops, no dealoc
    }
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn __heap_pos() -> usize {
    extern "C" {
        static __heap_base: u8;
    }
    let mut heap_pos = unsafe { HEAP_POS };
    heap_pos = unsafe { (&__heap_base) as *const u8 as usize };
    heap_pos
}
