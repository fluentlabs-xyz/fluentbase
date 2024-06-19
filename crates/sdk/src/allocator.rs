use core::alloc::{GlobalAlloc, Layout};

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
    let ptr = heap_pos as *mut u8;
    heap_pos += bytes;
    unsafe { HEAP_POS = heap_pos };
    ptr
}

#[inline(always)]
pub fn alloc_ptr(len: usize) -> *mut u8 {
    use alloc::alloc::{alloc, Layout};
    unsafe { alloc(Layout::from_size_align_unchecked(len, 8)) }
}

#[inline(always)]
pub fn alloc_slice<'a>(len: usize) -> &'a mut [u8] {
    use core::ptr;
    unsafe { &mut *ptr::slice_from_raw_parts_mut(alloc_ptr(len), len) }
}

#[cfg(target_arch = "wasm32")]
pub struct HeapBaseAllocator {}

#[cfg(target_arch = "wasm32")]
unsafe impl GlobalAlloc for HeapBaseAllocator {
    #[inline(always)]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        _sys_alloc_aligned(layout.size(), layout.align())
    }

    #[inline(always)]
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // ops, no dealoc
    }
}
