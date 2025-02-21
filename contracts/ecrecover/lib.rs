#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;

use fluentbase_sdk::{alloc_slice, func_entrypoint, Bytes, SharedAPI, FUEL_DENOM_RATE};

pub fn main(mut sdk: impl SharedAPI) {
    // read full input data
    let input_length = sdk.input_size();
    let mut input = alloc_slice(input_length as usize);
    sdk.read(&mut input, 0);
    let input = Bytes::copy_from_slice(input);
    // call identity function
    let gas_limit = sdk.fuel() / FUEL_DENOM_RATE;
    let result = revm_precompile::secp256k1::ec_recover_run(&input, gas_limit)
        .unwrap_or_else(|_| panic!("identity: precompile execution failed"));
    // write output
    sdk.write(result.bytes.as_ref());
}

func_entrypoint!(main);

#[cfg(test)]
mod tests {}
