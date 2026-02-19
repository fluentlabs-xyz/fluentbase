use crate::allocator::calc_pages_needed;

pub struct HeapBaseAllocator;

unsafe impl core::alloc::GlobalAlloc for HeapBaseAllocator {
    #[inline(never)]
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let bytes: usize = layout.size();
        let align: usize = layout.align();
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
            heap_pos = heap_pos.wrapping_add(align - offset);
        }
        // allocate memory pages if needed
        let pages_allocated = core::arch::wasm32::memory_size::<0>();
        let pages_needed = calc_pages_needed(pages_allocated, heap_pos + bytes);
        if pages_needed > 0 {
            let new_pages = core::arch::wasm32::memory_grow::<0>(pages_needed);
            if new_pages == usize::MAX {
                use fluentbase_types::{bindings::_exit, ExitCode};
                _exit(ExitCode::OutOfMemory.into_i32());
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
