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

// Nitro attestation documents are bounded by their schema: this verifier caps
// public_key at 1024 bytes, user_data and nonce at 512 bytes each, PCR count at
// 32, and cabundle certs at 1024 bytes each. The bundled real fixtures are
// below 5 KiB, so 16 KiB leaves operational headroom while bounding the work
// the parsers do on input that is not authenticated yet. The system entrypoint
// has already copied the input by the time this cap is checked.
const NITRO_MAX_INPUT_SIZE: u32 = 16 * 1024;

/// Verifies an AWS Nitro attestation document.
///
/// This runtime is engine-metered (`ENGINE_METERED_PRECOMPILES`): the execution engine charges
/// every instruction of the verification, so no static fee is charged on top of that.
///
/// Every rejection, an oversized input included, reverts with a `nitro: ...` reason and returns
/// the unused gas. A halt would burn all the gas a caller forwarded on input it may be relaying
/// for somebody else.
pub fn main_entry<SDK: SystemAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    let result = if sdk.input_size() > NITRO_MAX_INPUT_SIZE {
        Err("nitro: input too large")
    } else {
        let input = sdk.bytes_input();
        let current_timestamp = sdk.context().block_timestamp();
        parse_attestation_and_verify(input.as_ref(), current_timestamp).map(|_| ())
    };

    if let Err(err) = result {
        write_evm_panic_message(err, |slice| {
            sdk.write(slice);
        });
        return Err(ExitCode::Panic);
    }
    Ok(())
}

system_entrypoint!(main_entry);
