use alloc::vec::Vec;
use crate::debug_log;

const WASM_PAGE_SIZE_IN_BYTES: usize = 65536;
const HEAP_POS_CHECKPOINTS_HEIGHT_MIN: usize = 0;
const HEAP_POS_CHECKPOINTS_HEIGHT_MAX: usize = 1024;
static mut HEAP_POS_CHECKPOINTS: [usize; HEAP_POS_CHECKPOINTS_HEIGHT_MAX] = [0usize; HEAP_POS_CHECKPOINTS_HEIGHT_MAX];
static mut HEAP_POS_CHECKPOINTS_HEIGHT: usize = 0;

#[inline(always)]
pub fn checkpoint_count() -> usize {
    #[cfg(target_arch = "wasm32")]
    {
        unsafe {
            return HEAP_POS_CHECKPOINTS_HEIGHT
        }
    }
    usize::MAX
}

#[inline(always)]
pub fn checkpoint_try_save(skip_if_same_height: bool) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        unsafe {
            if HEAP_POS_CHECKPOINTS_HEIGHT >= HEAP_POS_CHECKPOINTS_HEIGHT_MAX {
                return false;
            }
            let current = alloc_heap_pos();
            if skip_if_same_height && HEAP_POS_CHECKPOINTS_HEIGHT > 0 {
                let prev = HEAP_POS_CHECKPOINTS[HEAP_POS_CHECKPOINTS_HEIGHT - 1];
                if prev == current {
                    return false;
                }
            }
            HEAP_POS_CHECKPOINTS[HEAP_POS_CHECKPOINTS_HEIGHT] = current;
            HEAP_POS_CHECKPOINTS_HEIGHT += 1;
        }
        return true
    }
    false
}

#[inline(always)]
pub fn checkpoint_try_restore(pop: bool) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        unsafe {
            if HEAP_POS_CHECKPOINTS_HEIGHT <= HEAP_POS_CHECKPOINTS_HEIGHT_MIN {
                return false;
            }
            let prev = HEAP_POS_CHECKPOINTS_HEIGHT - 1;
            let restored = HEAP_POS_CHECKPOINTS[prev];
            let rollback_ok = try_rollback_heap_pos(restored);
            if !rollback_ok {
                return false;
            } else if pop {
                HEAP_POS_CHECKPOINTS_HEIGHT = prev;
            }
        }
        return true
    }
    false
}

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
#[cfg(target_arch = "wasm32")]
static mut HEAP_POS_LAST: usize = 0;

#[inline(always)]
pub fn heap_pos_change() -> usize {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        if HEAP_POS > HEAP_POS_LAST {
            let change = HEAP_POS - HEAP_POS_LAST;
            HEAP_POS_LAST = HEAP_POS;
            return change;
        }
    }
    0
}

#[inline(always)]
pub fn heap_pos_last_reset(v: usize) {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        HEAP_POS_LAST = v;
    }
}

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
pub fn try_rollback_heap_pos(new_heap_pos: usize) -> bool {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        if new_heap_pos < HEAP_POS {
            HEAP_POS = new_heap_pos;
            heap_pos_last_reset(new_heap_pos);
            return true;
        }
    }
    false
}

pub struct HeapController {
    heap_pos: usize,
}
impl Drop for HeapController {
    fn drop(&mut self) {
        try_rollback_heap_pos(self.heap_pos);
        debug_log!("heap_pos rolled to {}", self.heap_pos);
    }
}
impl HeapController {
    pub fn new() -> Self {
        let heap_pos = alloc_heap_pos();
        debug_log!("heap_pos memorised {}", heap_pos);
        Self { heap_pos }
    }
    pub fn run_with_heap_drop<T, F: FnMut() -> T>(mut f: F) -> (bool, T) {
        let start = alloc_heap_pos();
        let r = f();
        // let stop = alloc_heap_pos();
        // debug_log!("start={} stop={} rolled_back={}", start, stop, stop-start);
        (try_rollback_heap_pos(start), r)
    }

    #[inline(always)]
    pub fn stack_pointer_offset() -> usize {
        #[cfg(target_arch = "wasm32")]
        unsafe {
            let some_var: u8 = 1;
            let offset = &some_var as *const u8 as usize;
            return offset;
        }
        0
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
