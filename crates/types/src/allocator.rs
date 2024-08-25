use alloc::vec::Vec;
#[cfg(target_arch = "wasm32")]
use core::alloc::GlobalAlloc;

const WASM_PAGE_SIZE_IN_BYTES: usize = 65536;

fn _calc_pages_needed(pages_allocated: usize, ptr: usize) -> usize {
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
    assert_eq!(_calc_pages_needed(0, 1), 1);
    assert_eq!(_calc_pages_needed(0, 65535), 1);
    assert_eq!(_calc_pages_needed(0, 65536), 2);
    assert_eq!(_calc_pages_needed(1, 65536), 2);
    assert_eq!(_calc_pages_needed(1, 65535), 0);
    assert_eq!(_calc_pages_needed(1, 65536 * 2), 3);
    assert_eq!(_calc_pages_needed(5, 327680), 6);
}

#[cfg(target_arch = "wasm32")]
fn _sys_alloc_aligned(bytes: usize, align: usize) -> *mut u8 {
    extern "C" {
        static __heap_base: u8;
    }
    static mut HEAP_POS: usize = 0;
    let mut heap_pos = unsafe { HEAP_POS };
    if heap_pos == 0 {
        heap_pos = unsafe { (&__heap_base) as *const u8 as usize };
    }
    let offset = heap_pos & (align - 1);
    if offset != 0 {
        heap_pos += align - offset;
    }
    // allocate memory pages if needed
    const WASM_PAGE_SIZE_IN_BYTES: usize = 65536;
    let pages_allocated = core::arch::wasm32::memory_size::<0>();
    let pages_needed = _calc_pages_needed(pages_allocated, heap_pos + bytes);
    if pages_needed > 0 {
        let new_pages = core::arch::wasm32::memory_grow::<0>(pages_needed);
        if new_pages == usize::MAX {
            unreachable!("out of memory");
        }
    }
    // return allocated pointer
    let ptr = heap_pos as *mut u8;
    heap_pos += bytes;
    unsafe { HEAP_POS = heap_pos };
    ptr
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
unsafe impl GlobalAlloc for HeapBaseAllocator {
    #[inline(always)]
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        _sys_alloc_aligned(layout.size(), layout.align())
    }

    #[inline(always)]
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {
        // ops, no dealoc
    }
}
