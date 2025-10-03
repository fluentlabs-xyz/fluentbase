#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{entrypoint, SharedAPI};

#[inline(always)]
pub fn fib(n: i32) -> i32 {
    let (mut a, mut b) = (0, 1);
    for _ in 0..n {
        let temp = a;
        a = b;
        b = temp + b;
    }
    a
}

pub fn main_entry(mut sdk: impl SharedAPI) {
    let input = sdk.input();
    if input.len() != 36 {
        panic!("input malformed: expected exactly 36 bytes: 4 bytes selector and 32 bytes value");
    }

    let n = i32::from_be_bytes([input[32], input[33], input[34], input[35]]);

    let res = fib(n);
    sdk.write(&res.to_be_bytes());
}

entrypoint!(main_entry);
