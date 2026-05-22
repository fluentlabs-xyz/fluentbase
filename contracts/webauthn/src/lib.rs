#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate fluentbase_sdk;

mod webauthn;

use fluentbase_sdk::{
    codec::SolidityABI, system_entrypoint, Bytes, ContextReader, ExitCode, SystemAPI, B256, U256,
};
use webauthn::{verify_webauthn, WebAuthnAuth};

/// Function selector: 0x94516dde
/// Derived from:
/// keccak256("verify(bytes,bool,(bytes,bytes,uint256,uint256,bytes32,bytes32),uint256,uint256)")
const VERIFY_SELECTOR: [u8; 4] = [0x94, 0x51, 0x6d, 0xde];

/// Estimated verification cost, in EVM gas units.
const WEBAUTHN_VERIFY_GAS: u64 = 22_000;

/// WebAuthn verification contract for blockchain authentication.
///
/// Based on reference implementations:
/// - Solady: https://github.com/vectorized/solady/blob/main/src/utils/WebAuthn.sol
/// - Daimo: https://github.com/daimo-eth/p256-verifier/blob/master/src/WebAuthn.sol
/// - Coinbase: https://github.com/base-org/webauthn-sol/blob/main/src/WebAuthn.sol
pub fn main_entry<SDK: SystemAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    sdk.sync_evm_gas(WEBAUTHN_VERIFY_GAS)?;

    if sdk.input_size() < 4 {
        return Err(ExitCode::MalformedBuiltinParams);
    }

    let input = sdk.bytes_input();
    let (selector, params) = input.split_at(4);
    if selector != VERIFY_SELECTOR {
        return Err(ExitCode::MalformedBuiltinParams);
    }

    let (challenge, require_user_verification, auth, x, y) =
        SolidityABI::<(Bytes, bool, WebAuthnAuth, U256, U256)>::decode(&params, 0)
            .map_err(|_| ExitCode::MalformedBuiltinParams)?;

    let gas_limit = sdk.context().contract_gas_limit();
    let is_valid = verify_webauthn(
        &challenge,
        require_user_verification,
        &auth,
        x,
        y,
        gas_limit,
    )?;
    let result = B256::with_last_byte(if is_valid { 0x01 } else { 0x00 });
    sdk.write(&result[..]);

    Ok(())
}

system_entrypoint!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{
        codec::bytes::BytesMut, crypto::crypto_sha256, Bytes, ContractContextV1, FUEL_DENOM_RATE,
    };
    use fluentbase_testing::TestingContextImpl;
    use p256::{
        ecdsa::{signature::SignerMut, SigningKey, VerifyingKey},
        elliptic_curve::rand_core::OsRng,
    };

    fn valid_call_params(
        require_user_verification: bool,
    ) -> (Bytes, bool, WebAuthnAuth, U256, U256) {
        let challenge = Bytes::copy_from_slice(
            &hex::decode("f631058a3ba1116acce12396fad0a125b5041c43f8e15723709f81aa8d5f4ccf")
                .unwrap(),
        );
        let authenticator_data = Bytes::copy_from_slice(
            &hex::decode(
                "49960de5880e8c687434170f6476605b8fe4aeb9a28632c7995cf3ba831d97630500000101",
            )
            .unwrap(),
        );
        let client_data_json = Bytes::copy_from_slice(
            format!(
                "{{\"type\":\"webauthn.get\",\"challenge\":\"{}\",\"origin\":\"http://localhost:3005\"}}",
                webauthn::base64url_encode(&challenge)
            )
            .as_bytes(),
        );

        let mut signing_key = SigningKey::random(&mut OsRng);
        let verifying_key = VerifyingKey::from(&signing_key);
        let public_key = verifying_key.to_encoded_point(false);
        let public_key = public_key.as_bytes();

        let client_data_hash = crypto_sha256(&client_data_json).0;
        let mut signed_message =
            Vec::with_capacity(authenticator_data.len() + client_data_hash.len());
        signed_message.extend_from_slice(&authenticator_data);
        signed_message.extend_from_slice(&client_data_hash);
        let signature: p256::ecdsa::Signature = signing_key.sign(&signed_message);
        let signature = signature.to_bytes();

        (
            challenge,
            require_user_verification,
            WebAuthnAuth {
                authenticator_data,
                client_data_json,
                challenge_index: U256::from(23),
                type_index: U256::from(1),
                r: U256::from_be_slice(&signature[..32]),
                s: U256::from_be_slice(&signature[32..]),
            },
            U256::from_be_slice(&public_key[1..33]),
            U256::from_be_slice(&public_key[33..65]),
        )
    }

    fn encode_call(params_tuple: &(Bytes, bool, WebAuthnAuth, U256, U256)) -> Vec<u8> {
        let mut params = BytesMut::new();
        SolidityABI::<(Bytes, bool, WebAuthnAuth, U256, U256)>::encode(
            params_tuple,
            &mut params,
            0,
        )
        .unwrap();

        let mut input = VERIFY_SELECTOR.to_vec();
        input.extend_from_slice(&params);
        input
    }

    fn valid_call_input(require_user_verification: bool) -> Vec<u8> {
        encode_call(&valid_call_params(require_user_verification))
    }

    fn exec(input: &[u8], gas_limit: u64) -> Result<(Vec<u8>, u64), ExitCode> {
        let mut sdk = TestingContextImpl::default()
            .with_input(Bytes::copy_from_slice(input))
            .with_contract_context(ContractContextV1 {
                gas_limit,
                ..Default::default()
            })
            .with_gas_limit(gas_limit);
        let result = main_entry(&mut sdk);
        Ok((sdk.take_output(), sdk.consumed_fuel())).and_then(|success| result.map(|_| success))
    }

    #[test]
    fn valid_signature_returns_true_and_charges_fuel() {
        let (output, fuel) = exec(&valid_call_input(true), WEBAUTHN_VERIFY_GAS).unwrap();
        assert_eq!(output, B256::with_last_byte(1)[..]);
        assert_eq!(fuel, WEBAUTHN_VERIFY_GAS * FUEL_DENOM_RATE);
    }

    #[test]
    fn missing_user_verification_returns_false_when_required() {
        let mut params = valid_call_params(true);
        let mut authenticator_data = params.2.authenticator_data.to_vec();
        authenticator_data[webauthn::AUTH_DATA_FLAGS_INDEX] = webauthn::AUTH_DATA_FLAGS_UP;
        params.2.authenticator_data = Bytes::copy_from_slice(&authenticator_data);

        let (output, fuel) = exec(&encode_call(&params), WEBAUTHN_VERIFY_GAS).unwrap();
        assert_eq!(output, B256::default()[..]);
        assert_eq!(fuel, WEBAUTHN_VERIFY_GAS * FUEL_DENOM_RATE);
    }

    #[test]
    fn malformed_selector_is_rejected_after_fuel_charge() {
        let mut input = valid_call_input(true);
        input[0] ^= 0xff;

        let err = exec(&input, WEBAUTHN_VERIFY_GAS).unwrap_err();
        assert_eq!(err, ExitCode::MalformedBuiltinParams);
    }

    #[test]
    fn insufficient_fuel_fails_before_verification() {
        let err = exec(&valid_call_input(true), WEBAUTHN_VERIFY_GAS - 1).unwrap_err();
        assert_eq!(err, ExitCode::OutOfFuel);
    }

    #[test]
    #[ignore = "local fuel estimate benchmark"]
    fn bench_webauthn_fuel_estimate() {
        let valid_input = valid_call_input(true);
        let mut invalid_params = valid_call_params(true);
        invalid_params.2.r = U256::ZERO;
        invalid_params.2.s = U256::ZERO;
        let invalid_input = encode_call(&invalid_params);

        for (name, input) in [("valid", valid_input), ("invalid_signature", invalid_input)] {
            let started = std::time::Instant::now();
            let iterations = 100u32;
            for _ in 0..iterations {
                let (_, fuel) = exec(&input, WEBAUTHN_VERIFY_GAS).unwrap();
                assert_eq!(fuel, WEBAUTHN_VERIFY_GAS * FUEL_DENOM_RATE);
            }
            println!(
                "webauthn {name}: gas={WEBAUTHN_VERIFY_GAS}, fuel={}, iterations={iterations}, elapsed={:?}",
                WEBAUTHN_VERIFY_GAS * FUEL_DENOM_RATE,
                started.elapsed()
            );
        }
    }
}
