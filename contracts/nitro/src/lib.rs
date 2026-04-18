#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
#![allow(clippy::useless_conversion, clippy::vec_init_then_push)]
#![allow(dead_code)]

use fluentbase_sdk::{system_entrypoint, ExitCode, SystemAPI};

pub fn main_entry<SDK: SystemAPI>(_sdk: &mut SDK) -> Result<(), ExitCode> {
    // Temporarily disabled
    Err(ExitCode::UnreachableCodeReached)
}

// extern crate alloc;
// extern crate fluentbase_sdk;
//
// mod attestation;
// #[cfg(test)]
// mod tests;
//
// use crate::attestation::parse_attestation_and_verify;
// use fluentbase_sdk::{
//     evm::write_evm_panic_message, system_entrypoint, ContextReader, ExitCode, SystemAPI,
// };
//
// pub fn main_entry<SDK: SystemAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
//     let input = sdk.bytes_input();
//     let current_timestamp = sdk.context().block_timestamp();
//     if let Err(err) = parse_attestation_and_verify(input.as_ref(), current_timestamp) {
//         // write an EVM-compatible panic message
//         write_evm_panic_message(err, |slice| {
//             sdk.write(slice);
//         });
//         return Err(ExitCode::Panic);
//     }
//     Ok(())
// }

system_entrypoint!(main_entry);
