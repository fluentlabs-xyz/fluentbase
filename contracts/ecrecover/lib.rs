#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;

use fluentbase_sdk::B256;
use fluentbase_sdk::{alloc_slice, entrypoint, Bytes, ContextReader, ExitCode, SharedAPI};

fn ecrecover_with_sdk<SDK: SharedAPI>(
    _: &SDK,
    digest: &B256,
    sig: &[u8; 64],
    rec_id: u8,
) -> Option<[u8; 65]> {
    SDK::secp256k1_recover(digest, sig, rec_id)
}

pub fn main_entry(mut sdk: impl SharedAPI) {
    // read full input data
    let gas_limit = sdk.context().contract_gas_limit();
    let input_length = sdk.input_size();
    let mut input = alloc_slice(input_length as usize);
    sdk.read(&mut input, 0);

    let input = Bytes::copy_from_slice(input);
    let gas_used = estimate_gas(input.len());
    if gas_used > gas_limit {
        sdk.native_exit(ExitCode::OutOfFuel);
    }

    // EVM ecrecover input is 4 words (32 bytes each): hash, v, r, s.
    // Pad/truncate input to 128 bytes as per EVM behavior.
    let mut data = [0u8; 128];
    let to_copy = core::cmp::min(128, input.len());
    data[..to_copy].copy_from_slice(&input[..to_copy]);

    // Parse fields
    let digest = B256::from_slice(&data[0..32]);

    // v is 32-byte big-endian integer; require top 31 bytes to be zero, AND v must be 27 or 28
    let v_bytes = &data[32..64];
    if !(v_bytes[..31].iter().all(|&b| b == 0) && matches!(v_bytes[31], 27 | 28)) {
        // Invalid v, return empty
        sdk.sync_evm_gas(gas_used, 0);
        sdk.write(&[]);
        return;
    }
    let v = v_bytes[31] - 27;

    // r and s
    let r = &data[64..96];
    let s = &data[96..128];
    let mut sig = [0u8; 64];
    sig[..32].copy_from_slice(r);
    sig[32..].copy_from_slice(s);

    // Perform recover using SDK
    let pubkey = match ecrecover_with_sdk(&sdk, &digest, &sig, v) {
        Some(pk) => pk,
        None => {
            sdk.sync_evm_gas(gas_used, 0);
            sdk.write(&[]);
            return;
        }
    };

    // Compute address = last 20 bytes of keccak256(uncompressed_pubkey[1...])
    // SDK returns 65-byte uncompressed pubkey [0x04 || x || y]
    let hashed = sdk.keccak256(&pubkey[1..65]);
    let mut out = [0u8; 32];
    out[12..32].copy_from_slice(&hashed[12..32]);

    sdk.sync_evm_gas(gas_used, 0);
    sdk.write(&out);
}

// Gas estimation for ECRECOVER (based on EVM gas model)
fn estimate_gas(_input_len: usize) -> u64 {
    // ECRECOVER precompile has a fixed cost of 3000 gas(const ECRECOVER_BASE: u64 = 3_000;)
    3000
}

entrypoint!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{hex, ContractContextV1, FUEL_DENOM_RATE};
    use fluentbase_sdk_testing::HostTestingContext;

    fn exec_evm_precompile(inputs: &[u8], expected: &[u8], expected_gas: u64) {
        let gas_limit = 100_000;
        let sdk = HostTestingContext::default()
            .with_input(Bytes::copy_from_slice(inputs))
            .with_contract_context(ContractContextV1 {
                gas_limit,
                ..Default::default()
            })
            .with_gas_limit(gas_limit);
        main_entry(sdk.clone());
        let output = sdk.take_output();
        assert_eq!(output, expected);
        let gas_remaining = sdk.fuel() / FUEL_DENOM_RATE;
        assert_eq!(gas_limit - gas_remaining, expected_gas);
    }

    #[test]
    fn call_ecrecover_unrecoverable_key() {
        exec_evm_precompile(
            &hex!("a8b53bdf3306a35a7103ab5504a0c9b492295564b6202b1942a84ef300107281000000000000000000000000000000000000000000000000000000000000001b307835653165303366353363653138623737326363623030393366663731663366353366356337356237346463623331613835616138623838393262346538621122334455667788991011121314151617181920212223242526272829303132"),
            &hex!(""),
            3000,
        );
    }

    #[test]
    fn valid_key() {
        exec_evm_precompile(
            &hex!("18c547e4f7b0f325ad1e56f57e26c745b09a3e503d86e00e5255ff7f715d3d1c000000000000000000000000000000000000000000000000000000000000001c73b1693892219d736caba55bdb67216e485557ea6b6af75f37096c9aa6a5a75feeb940b1d03b21e36b0e47e79769f095fe2ab855bd91e3a38756b7d75a9c4549"),
            &hex!("000000000000000000000000a94f5374fce5edbc8e2a8697c15331677e6ebf0b"),
            3000,
        );
    }

    #[test]
    fn invalid_high_v_bits_1() {
        exec_evm_precompile(
            &hex!("18c547e4f7b0f325ad1e56f57e26c745b09a3e503d86e00e5255ff7f715d3d1c100000000000000000000000000000000000000000000000000000000000001c73b1693892219d736caba55bdb67216e485557ea6b6af75f37096c9aa6a5a75feeb940b1d03b21e36b0e47e79769f095fe2ab855bd91e3a38756b7d75a9c4549"),
            &hex!(""),
            3000,
        );
    }

    #[test]
    fn invalid_high_v_bits_2() {
        exec_evm_precompile(
            &hex!("18c547e4f7b0f325ad1e56f57e26c745b09a3e503d86e00e5255ff7f715d3d1c000000000000000000000000000000000000001000000000000000000000001c73b1693892219d736caba55bdb67216e485557ea6b6af75f37096c9aa6a5a75feeb940b1d03b21e36b0e47e79769f095fe2ab855bd91e3a38756b7d75a9c4549"),
            &hex!(""),
            3000,
        );
    }

    #[test]
    fn invalid_high_v_bits_3() {
        exec_evm_precompile(
            &hex!("18c547e4f7b0f325ad1e56f57e26c745b09a3e503d86e00e5255ff7f715d3d1c000000000000000000000000000000000000001000000000000000000000011c73b1693892219d736caba55bdb67216e485557ea6b6af75f37096c9aa6a5a75feeb940b1d03b21e36b0e47e79769f095fe2ab855bd91e3a38756b7d75a9c4549"),
            &hex!(""),
            3000,
        );
    }
}
