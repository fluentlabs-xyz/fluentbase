use crate::consts::U256_BYTES_COUNT;
use core::slice;

pub const SP_VAL_MEM_OFFSET_DEFAULT: usize = 1024 * 8;

pub(crate) fn sp_set(sp_mem_offset: usize, value: u64) {
    let mut mem: &mut [u64];
    unsafe {
        mem = slice::from_raw_parts_mut(sp_mem_offset as *mut u64, 1);
    }
    mem[0] = value;
}

pub(crate) fn sp_get(sp_val_mem_offset: usize) -> u64 {
    let mut mem: &mut [u64];
    unsafe {
        mem = slice::from_raw_parts_mut(sp_val_mem_offset as *mut u64, 1);
    }
    mem[0]
}

pub(crate) fn sp_compute_mem_offset(sp_val_mem_offset: usize, sp: u64) -> u64 {
    sp_val_mem_offset as u64 - sp
}

pub(crate) fn sp_get_mem_offset(sp_val_mem_offset: usize) -> u64 {
    let sp = sp_get(sp_val_mem_offset);
    sp_compute_mem_offset(sp_val_mem_offset, sp)
}

// TODO check boundaries?
pub(crate) fn sp_change(sp_val_mem_offset: usize, count_bytes: i32) -> u64 {
    let mut sp = sp_get(sp_val_mem_offset);
    sp = (sp as i32 + count_bytes) as u64;
    sp_set(sp_val_mem_offset, sp);

    sp
}

pub(crate) fn sp_inc(sp_val_mem_offset: usize, count_bytes: u64) -> u64 {
    sp_change(sp_val_mem_offset, count_bytes as i32)
}

pub(crate) fn sp_dec(sp_val_mem_offset: usize, count_bytes: u64) -> u64 {
    sp_change(sp_val_mem_offset, -(count_bytes as i32))
}

pub(crate) fn u256_push(sp_val_mem_offset: usize, val: [u8; U256_BYTES_COUNT as usize]) {
    let sp = sp_inc(sp_val_mem_offset, U256_BYTES_COUNT);
    let sp_mem_offset = sp_compute_mem_offset(sp_val_mem_offset, sp);

    let dest =
        unsafe { slice::from_raw_parts_mut(sp_mem_offset as *mut u8, U256_BYTES_COUNT as usize) };
    dest.copy_from_slice(&val);
}

pub(crate) fn u256_pop(sp_val_mem_offset: usize) -> [u8; U256_BYTES_COUNT as usize] {
    let mut res = [0u8; U256_BYTES_COUNT as usize];

    let sp_mem_offset = sp_get_mem_offset(sp_val_mem_offset);
    let v =
        unsafe { slice::from_raw_parts_mut(sp_mem_offset as *mut u8, U256_BYTES_COUNT as usize) };
    res.copy_from_slice(v);
    sp_dec(sp_val_mem_offset, U256_BYTES_COUNT);

    res
}
