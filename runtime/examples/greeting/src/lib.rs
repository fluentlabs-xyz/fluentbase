#![no_std]

use fluentbase_core::*;

#[no_mangle]
pub extern "C" fn main() {
    let mut input: [u8; 10] = [0; 10];
    let n = sys_read(input.as_mut_ptr(), 0, 10);
    if n < 10 {
        panic!("input not enough")
    }
    let sum = input.iter().fold(0u32, |r, v| r + *v as u32);
    let sum_bytes = sum.to_be_bytes();
    sys_write_slice(&sum_bytes, 0)
}
