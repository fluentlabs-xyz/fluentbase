#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{alloc_slice, func_entrypoint, Bytes, SharedAPI, FUEL_DENOM_RATE};

pub fn main(mut sdk: impl SharedAPI) {
    // read full input data
    let input_length = sdk.input_size();
    let mut input = alloc_slice(input_length as usize);
    sdk.read(&mut input, 0);
    let input = Bytes::copy_from_slice(input);
    // call ripemd160 function
    let gas_limit = sdk.fuel() / FUEL_DENOM_RATE;
    let result = revm_precompile::hash::ripemd160_run(&input, gas_limit)
        .unwrap_or_else(|_| panic!("ripemd160: precompile execution failed"));
    // write output
    sdk.write(result.bytes.as_ref());
}

func_entrypoint!(main);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{bytes, testing::TestingContext};

    #[test]
    fn test_hello_world_works() {
        let sdk = TestingContext::default().with_input("Hello, World");
        main(sdk.clone());
        let output = sdk.take_output();
        assert_eq!(
            output,
            bytes!("0000000000000000000000006782893f9a818abc3da35d745a803d72a660c9f5")
        )
    }
}
