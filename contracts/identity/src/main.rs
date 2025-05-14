#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;

use fluentbase_sdk::{alloc_slice, entrypoint, ContractContextReader, ExitCode, SharedAPI};
use revm_precompile::{
    calc_linear_cost_u32,
    identity::{IDENTITY_BASE, IDENTITY_PER_WORD},
};

pub fn main_entry(mut sdk: impl SharedAPI) {
    let gas_limit = sdk.context().contract_gas_limit();
    let input_length = sdk.input_size();
    // fail fast if we don't have enough fuel for the call
    let gas_used = calc_linear_cost_u32(input_length as usize, IDENTITY_BASE, IDENTITY_PER_WORD);
    if gas_used > gas_limit {
        sdk.exit(ExitCode::OutOfFuel);
    }
    sdk.sync_evm_gas(gas_used, 0);
    let mut input = alloc_slice(input_length as usize);
    sdk.read(&mut input, 0);
    // write an identical output
    sdk.write(input);
}

entrypoint!(main_entry);
