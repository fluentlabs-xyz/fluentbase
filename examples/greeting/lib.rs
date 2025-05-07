#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{func_entrypoint, SharedAPI};

#[allow(dead_code)]
fn fib(n: i32) -> i32 {
    if n <= 0 {
        0
    } else if n == 1 {
        1
    } else {
        fib(n - 1) + fib(n - 2)
    }
}

pub fn main(mut sdk: impl SharedAPI) {
    // calculate a fib of 47
    // fib(core::hint::black_box(47));
    // write "Hello, World" message into output
    sdk.write("Hello, World".as_bytes());
}

func_entrypoint!(main);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk_testing::HostTestingContext;

    #[test]
    fn test_contract_works() {
        let sdk = HostTestingContext::default();
        main(sdk.clone());
        let output = sdk.take_output();
        assert_eq!(&output, "Hello, World".as_bytes());
    }
}
