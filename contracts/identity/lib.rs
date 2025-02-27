#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;

use fluentbase_sdk::{alloc_slice, func_entrypoint, SharedAPI, FUEL_DENOM_RATE};
use revm_precompile::{
    calc_linear_cost_u32,
    identity::{IDENTITY_BASE, IDENTITY_PER_WORD},
};

pub fn main(mut sdk: impl SharedAPI) {
    let input_length = sdk.input_size();
    // fail fast if we don't have enough fuel for the call
    let gas_used = calc_linear_cost_u32(input_length as usize, IDENTITY_BASE, IDENTITY_PER_WORD);
    if gas_used > sdk.fuel() / FUEL_DENOM_RATE {
        sdk.charge_fuel(u64::MAX);
    }
    let mut input = alloc_slice(input_length as usize);
    sdk.read(&mut input, 0);
    // write an identical output
    sdk.write(input);
}

func_entrypoint!(main);

#[cfg(test)]
mod tests {
    use super::*;
    use core::str::from_utf8;
    use fluentbase_sdk::testing::TestingContext;

    #[test]
    fn test_hello_world_works() {
        let sdk = TestingContext::default().with_input("Hello, World");
        main(sdk.clone());
        let output = sdk.take_output();
        assert_eq!(from_utf8(&output).unwrap(), "Hello, World")
    }
}
