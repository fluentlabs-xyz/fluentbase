#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
#![allow(clippy::useless_conversion, clippy::vec_init_then_push)]
#![allow(dead_code)]

extern crate alloc;
extern crate fluentbase_sdk;

mod attestation;
#[cfg(test)]
mod tests;

use crate::attestation::{parse_attestation_and_verify, AttestationDoc, MAX_PCR_INDEX};
use alloc::{string::String, vec, vec::Vec};
use fluentbase_sdk::{
    codec::{bytes::BytesMut, SolidityABI},
    evm::write_evm_panic_message,
    system_entrypoint, Bytes, ContextReader, ExitCode, SystemAPI,
};

// Nitro attestation documents are bounded by their schema: this verifier caps
// public_key at 1024 bytes, user_data and nonce at 512 bytes each, PCR count at
// 32, and cabundle certs at 1024 bytes each. The bundled real fixtures are
// below 5 KiB, so 16 KiB leaves operational headroom while bounding the work
// the parsers do on input that is not authenticated yet. The system entrypoint
// has already copied the input by the time this cap is checked.
const NITRO_MAX_INPUT_SIZE: u32 = 16 * 1024;

/// The fields of a verified document a caller checks against its policy, returned as
/// `abi.encode(string moduleId, uint64 timestamp, bytes[] pcrs, bytes publicKey, bytes userData,
/// bytes nonce)`.
type VerifiedFields = (String, u64, Vec<Bytes>, Bytes, Bytes, Bytes);

/// Verifies an AWS Nitro attestation document and returns its [`VerifiedFields`].
///
/// `timestamp` is the document's, in milliseconds. `pcrs[i]` is PCR `i`, and empty where the
/// document has no PCR `i`, which a real PCR never is (32, 48 or 64 bytes). An absent or null
/// `public_key`, `user_data` or `nonce` is returned as empty bytes.
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
        parse_attestation_and_verify(input.as_ref(), current_timestamp).and_then(encode_fields)
    };

    match result {
        Ok(output) => {
            sdk.write(&output);
            Ok(())
        }
        Err(err) => {
            write_evm_panic_message(err, |slice| {
                sdk.write(slice);
            });
            Err(ExitCode::Panic)
        }
    }
}

fn encode_fields(doc: AttestationDoc) -> Result<BytesMut, &'static str> {
    let mut pcrs = vec![Bytes::new(); MAX_PCR_INDEX as usize + 1];
    let mut len = 0;
    for (index, value) in doc.pcrs {
        let index = usize::try_from(index).map_err(|_| "nitro: invalid pcr index")?;
        let slot = pcrs.get_mut(index).ok_or("nitro: invalid pcr index")?;
        *slot = value.into();
        len = len.max(index + 1);
    }
    pcrs.truncate(len);
    let fields: VerifiedFields = (
        doc.module_id,
        doc.timestamp,
        pcrs,
        doc.public_key.unwrap_or_default().into(),
        doc.user_data.unwrap_or_default().into(),
        doc.nonce.unwrap_or_default().into(),
    );
    let mut output = BytesMut::new();
    SolidityABI::encode_function_args(&fields, &mut output)
        .map_err(|_| "nitro: output encoding failed")?;
    Ok(output)
}

system_entrypoint!(main_entry);
