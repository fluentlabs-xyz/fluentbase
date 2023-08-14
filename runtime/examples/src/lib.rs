#![no_std]

use fluentbase_core::*;

fn greeting() {
    let mut input: [u8; 10] = [0; 10];
    let n = sys_read(input.as_mut_ptr(), 0, 10);
    let sum = input.iter().fold(0u32, |r, v| r + *v as u32);
    let sum_bytes = sum.to_be_bytes();
    evm_return_slice(&sum_bytes)
}

fn panic() {
    panic!("its time to panic");
}

#[no_mangle]
pub extern "C" fn main() {
    #[cfg(feature = "greeting")]
    greeting();
    #[cfg(feature = "panic")]
    panic();
}
