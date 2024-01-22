use crate::consts::U256_BYTES_COUNT;
use core::slice;

pub const SP_BASE_MEM_OFFSET_DEFAULT: usize = 1024 * 32;

// extern "C" {
//     fn __get_stack_pointer() -> u32;
//     fn __set_stack_pointer(v: i32) -> ();
// }
//
// pub(crate) fn get_stack_pointer() -> i32 {
//     unsafe { __get_stack_pointer() as i32 }
// }
//
// pub(crate) fn set_stack_pointer(v: i32) -> () {
//     unsafe { __set_stack_pointer(v) }
// }

pub(crate) fn sp_set(sp_base_mem_offset: usize, value: u64) {
    unsafe { *(sp_base_mem_offset as *mut u64) = value }
}

pub(crate) fn sp_get(sp_base_mem_offset: usize) -> u64 {
    unsafe { *(sp_base_mem_offset as *mut u64) }
}

pub(crate) fn sp_compute_mem_offset(sp_base_mem_offset: usize, sp: u64) -> u64 {
    sp_base_mem_offset as u64 - sp
}

pub(crate) fn sp_get_mem_offset(sp_base_mem_offset: usize) -> u64 {
    let sp = sp_get(sp_base_mem_offset);
    sp_compute_mem_offset(sp_base_mem_offset, sp)
}

// TODO check boundaries?
pub(crate) fn sp_change(sp_base_mem_offset: usize, count_bytes: i32) -> u64 {
    let mut sp = sp_get(sp_base_mem_offset);
    sp = (sp as i32 + count_bytes) as u64;
    sp_set(sp_base_mem_offset, sp);

    sp
}

pub(crate) fn sp_inc(sp_base_mem_offset: usize, count_bytes: u64) -> u64 {
    sp_change(sp_base_mem_offset, count_bytes as i32)
}

pub(crate) fn sp_dec(sp_base_mem_offset: usize, count_bytes: u64) -> u64 {
    sp_change(sp_base_mem_offset, -(count_bytes as i32))
}

pub(crate) fn stack_push_u256(sp_base_mem_offset: usize, val: [u8; U256_BYTES_COUNT as usize]) {
    let sp = sp_inc(sp_base_mem_offset, U256_BYTES_COUNT);
    let sp_mem_offset = sp_compute_mem_offset(sp_base_mem_offset, sp);

    let dest =
        unsafe { slice::from_raw_parts_mut(sp_mem_offset as *mut u8, U256_BYTES_COUNT as usize) };
    dest.copy_from_slice(&val);
}

pub(crate) fn stack_pop_u256(sp_base_mem_offset: usize) -> [u8; U256_BYTES_COUNT as usize] {
    let sp_mem_offset = sp_get_mem_offset(sp_base_mem_offset);
    let v =
        unsafe { slice::from_raw_parts_mut(sp_mem_offset as *mut u8, U256_BYTES_COUNT as usize) };

    let mut res = [0u8; U256_BYTES_COUNT as usize];
    res.copy_from_slice(v);
    sp_dec(sp_base_mem_offset, U256_BYTES_COUNT);

    res
}

pub(crate) fn stack_peek_u256(
    sp_base_mem_offset: usize,
    param_idx: usize,
) -> [u8; U256_BYTES_COUNT as usize] {
    let sp_mem_offset = sp_get_mem_offset(sp_base_mem_offset);
    let src = unsafe {
        slice::from_raw_parts(
            (sp_mem_offset + param_idx as u64 * U256_BYTES_COUNT) as *const u8,
            U256_BYTES_COUNT as usize,
        )
    };
    let mut res = [0u8; U256_BYTES_COUNT as usize];
    res.copy_from_slice(src);

    res
}

pub(crate) fn stack_write_u256(
    sp_base_mem_offset: usize,
    val: &[u8; U256_BYTES_COUNT as usize],
    at_param_idx: usize,
) {
    let sp_mem_offset = sp_get_mem_offset(sp_base_mem_offset);
    let dest = unsafe {
        slice::from_raw_parts_mut(
            (sp_mem_offset + at_param_idx as u64 * U256_BYTES_COUNT) as *mut u8,
            U256_BYTES_COUNT as usize,
        )
    };
    dest.copy_from_slice(val);
}
