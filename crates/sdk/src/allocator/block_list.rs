use crate::allocator::calc_pages_needed;

#[repr(C)]
struct BlockHeader {
    prev: *mut BlockHeader,
    start: usize,
    end: usize,
    freed: u8,
    _pad: [u8; 7],
}

static mut HEAP_POS: usize = 0;

static mut HEAD: *mut BlockHeader = core::ptr::null_mut();

#[inline(always)]
unsafe fn align_up(x: usize, align: usize) -> usize {
    debug_assert!(align.is_power_of_two());
    (x + (align - 1)) & !(align - 1)
}

#[inline(always)]
unsafe fn ensure_pages(end: usize) {
    let pages_allocated = core::arch::wasm32::memory_size::<0>();
    let pages_needed = calc_pages_needed(pages_allocated, end);
    if pages_needed > 0 {
        let new_pages = core::arch::wasm32::memory_grow::<0>(pages_needed);
        if new_pages == usize::MAX {
            use fluentbase_types::{bindings::_exit, ExitCode};
            _exit(ExitCode::OutOfMemory.into_i32());
        }
    }
}

unsafe fn gc_pop_head() {
    // Rewind the bump pointer to start of this block
    // let head_before = (*HEAD).start;
    while !HEAD.is_null() && (*HEAD).freed != 0 {
        HEAP_POS = (*HEAD).start;
        HEAD = (*HEAD).prev;
    }
    // crate::debug_log!(
    //     "GC: new pop_head={:p}, head_before={}, new_pos={}",
    //     (*HEAD).start as *const u8,
    //     head_before,
    //     HEAP_POS
    // );
}

unsafe fn alloc_impl(layout: core::alloc::Layout) -> *mut u8 {
    let bytes = layout.size();
    let align = layout.align().max(core::mem::align_of::<usize>());

    // Init heap pos (if it's not set yet)
    let mut pos = HEAP_POS;
    if pos == 0 {
        extern "C" {
            static __heap_base: u8;
        }
        pos = (&__heap_base) as *const u8 as usize;
    }

    // We lay out as: [BlockHeader][padding to align payload][back ptr usize][payload bytes]
    let header_start = align_up(pos, core::mem::align_of::<BlockHeader>());
    let header_size = core::mem::size_of::<BlockHeader>();

    let after_header = header_start + header_size;

    // Payload must be aligned; we also reserve usize just before payload for back ptr
    let payload_start = align_up(after_header + core::mem::size_of::<usize>(), align);
    let back_ptr_addr = payload_start - core::mem::size_of::<usize>();
    let end = payload_start.checked_add(bytes).unwrap_or(usize::MAX);

    ensure_pages(end);

    // Write header
    let hdr = header_start as *mut BlockHeader;
    core::ptr::write(
        hdr,
        BlockHeader {
            prev: HEAD,
            start: header_start,
            end,
            freed: 0,
            _pad: [0; 7],
        },
    );

    // crate::debug_log!(
    //     "GC: new block ptr={:p}, size={}",
    //     hdr as *mut u8,
    //     end - header_start
    // );

    // Store back-pointer so dealloc can find the header
    core::ptr::write(back_ptr_addr as *mut usize, hdr as usize);

    // Advance allocator state
    HEAD = hdr;
    HEAP_POS = end;

    payload_start as *mut u8
}

unsafe fn dealloc_impl(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    let back_ptr_addr = (ptr as usize) - core::mem::size_of::<usize>();
    let hdr = core::ptr::read(back_ptr_addr as *const usize) as *mut BlockHeader;
    // crate::debug_log!(
    //     "GC: freeing block ptr={:p} size={}",
    //     hdr as *mut u8,
    //     (*hdr).end - (*hdr).start
    // );
    // Mark freed
    (*hdr).freed = 1;
}

pub struct BlockListAllocator;

unsafe impl core::alloc::GlobalAlloc for BlockListAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        if layout.size() == 0 {
            return layout.align() as *mut u8;
        }
        alloc_impl(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: core::alloc::Layout) {
        dealloc_impl(ptr);
        gc_pop_head();
    }
}

impl BlockListAllocator {
    #[cfg(feature = "debug-print")]
    pub fn dump_blocks() {
        unsafe {
            let mut it = HEAD;
            while !it.is_null() {
                crate::debug_log!(
                    "GC: block at ptr={:p}, size={}, is_free={}",
                    (*it).start as *mut u8,
                    (*it).end - (*it).start,
                    (*it).freed
                );
                it = (*it).prev;
            }
        }
    }

    pub fn gc() {
        unsafe { gc_pop_head() }
    }
}
