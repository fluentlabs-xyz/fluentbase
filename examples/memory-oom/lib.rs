#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{alloc_ptr, entrypoint, SharedAPI};

pub fn main_entry(_sdk: impl SharedAPI) {
    // Max allowed pages is 1024, then the max memory we can allocate is 67108864
    let ptr = alloc_ptr(67108864);
    core::hint::black_box(ptr);
}

entrypoint!(main_entry);
