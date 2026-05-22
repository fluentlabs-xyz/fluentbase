#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
#![allow(clippy::useless_conversion, clippy::vec_init_then_push)]
#![allow(dead_code)]

extern crate alloc;
extern crate fluentbase_sdk;

mod attestation;
#[cfg(test)]
mod tests;

use crate::attestation::parse_attestation_and_verify;
use fluentbase_sdk::{
    evm::write_evm_panic_message, system_entrypoint, ContextReader, ExitCode, SystemAPI,
};

/// Estimated Nitro attestation verification cost, in EVM gas units.
const NITRO_VERIFY_GAS: u64 = 1_250_000;

// Nitro attestation documents are bounded by their schema: this verifier caps
// public_key at 1024 bytes, user_data and nonce at 512 bytes each, PCR count at
// 32, and cabundle certs at 1024 bytes each. The bundled real fixtures are
// below 5 KiB, so 16 KiB leaves operational headroom while rejecting calldata
// large enough to force expensive allocation before parsing.
const NITRO_MAX_INPUT_SIZE: u32 = 16 * 1024;

pub fn main_entry<SDK: SystemAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    sdk.sync_evm_gas(NITRO_VERIFY_GAS)?;

    if sdk.input_size() > NITRO_MAX_INPUT_SIZE {
        return Err(ExitCode::MalformedBuiltinParams);
    }

    let input = sdk.bytes_input();
    let current_timestamp = sdk.context().block_timestamp();
    if let Err(err) = parse_attestation_and_verify(input.as_ref(), current_timestamp) {
        write_evm_panic_message(err, |slice| {
            sdk.write(slice);
        });
        return Err(ExitCode::Panic);
    }
    Ok(())
}

system_entrypoint!(main_entry);
