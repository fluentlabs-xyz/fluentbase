#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{entrypoint, SharedAPI};

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

pub fn main_entry(mut sdk: impl SharedAPI) {
    // calculate a fib of 47
    // fib(core::hint::black_box(47));
    // write "Hello, World" message into output
    sdk.write("Hello, World".as_bytes());
}

entrypoint!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk_testing::{include_this_wasm, run_with_default_context, HostTestingContext};
    const WASM_BYTECODE: &[u8] = include_this_wasm!();

    #[test]
    fn test_contract_works() {
        let sdk = HostTestingContext::default();
        main_entry(sdk.clone());
        let output = sdk.take_output();
        assert_eq!(&output, "Hello, World".as_bytes());
    }

    #[test]
    fn test_contract_works_in_emulated_environment() {
        let (output, exit_code) =
            run_with_default_context(WASM_BYTECODE.to_vec(), "Hello, World".as_bytes());
        assert_eq!(exit_code, 0);
        assert_eq!(output.clone(), "Hello, World".as_bytes().to_vec());
    }
}
