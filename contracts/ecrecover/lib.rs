#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;

use fluentbase_sdk::{alloc_slice, system_entrypoint2, Bytes, ExitCode, SharedAPI, B256, B512};
use revm_precompile::{secp256k1::ecrecover, utilities::right_pad};

pub fn main_entry<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    // read full input data
    let input_length = sdk.input_size();
    let mut input = alloc_slice(input_length as usize);
    sdk.read(&mut input, 0);
    let input = Bytes::copy_from_slice(input);

    // Make sure we have enough gas for execution
    const ECRECOVER_BASE: u64 = 3_000;
    sdk.sync_evm_gas(ECRECOVER_BASE)?;

    let input = right_pad::<128>(input.as_ref());

    // `v` must be a 32-byte big-endian integer equal to 27 or 28.
    if !(input[32..63].iter().all(|&b| b == 0) && matches!(input[63], 27 | 28)) {
        return Ok(());
    }

    let msg = <&B256>::try_from(&input[0..32]).unwrap();
    let rec_id = input[63] - 27;
    let sig = <&B512>::try_from(&input[64..128]).unwrap();

    if let Ok(result) = ecrecover(sig, rec_id, msg) {
        sdk.write(result);
    }
    Ok(())

    // TODO(dmitry123): Recover signature using ecdsa library once we have unconstrainted mode
    // let Ok(signature) = Signature::<k256::Secp256k1>::from_slice(&sig) else {
    //     return;
    // };
    // let recover_id = RecoveryId::from_byte(v).unwrap();
    // let Ok(public_key) =
    //     VerifyingKey::recover_from_prehash_secp256k1(digest.as_slice(), &signature, recover_id)
    // else {
    //     return;
    // };
    // let public_key = public_key.to_encoded_point(false);
    // let public_key = public_key.as_bytes();
    // Compute address = last 20 bytes of keccak256(uncompressed_pubkey[1...])
    // SDK returns 65-byte uncompressed pubkey [0x04 || x || y]
    // let hashed = CryptoRuntime::keccak256(&public_key[1..65]);
    // let mut out = [0u8; 32];
    // out[12..32].copy_from_slice(&hashed[12..32]);
    // sdk.write(&out);
}

system_entrypoint2!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{hex, ContractContextV1, FUEL_DENOM_RATE};
    use fluentbase_testing::TestingContextImpl;

    fn exec_evm_precompile(inputs: &[u8], expected: &[u8], expected_gas: u64) {
        let gas_limit = 100_000;
        let mut sdk = TestingContextImpl::default()
            .with_input(Bytes::copy_from_slice(inputs))
            .with_contract_context(ContractContextV1 {
                gas_limit,
                ..Default::default()
            })
            .with_gas_limit(gas_limit);
        main_entry(&mut sdk).unwrap();
        let output = sdk.take_output();
        assert_eq!(&output, expected);
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
