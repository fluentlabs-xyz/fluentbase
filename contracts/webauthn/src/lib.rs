#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate fluentbase_sdk;

mod client_data;
mod webauthn;

use fluentbase_sdk::{
    codec::SolidityABI, system_entrypoint, Bytes, ContextReader, ExitCode, SystemAPI, B256, U256,
};
use webauthn::{verify_webauthn, verify_webauthn_with_policy, WebAuthnAuth, WebAuthnPolicy};

/// Function selector: 0x94516dde
/// Derived from:
/// keccak256("verify(bytes,bool,(bytes,bytes,uint256,uint256,bytes32,bytes32),uint256,uint256)")
const VERIFY_SELECTOR: [u8; 4] = [0x94, 0x51, 0x6d, 0xde];

/// Function selector: 0x42520fdd
/// Derived from:
/// keccak256("verifyStrict(bytes,bool,bytes32,bytes,(bytes,bytes,uint256,uint256,bytes32,bytes32),uint256,uint256)")
const VERIFY_STRICT_SELECTOR: [u8; 4] = [0x42, 0x52, 0x0f, 0xdd];

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

    let gas_limit = sdk.context().contract_gas_limit();
    let is_valid = if selector == VERIFY_SELECTOR {
        let (challenge, require_user_verification, auth, x, y) =
            SolidityABI::<(Bytes, bool, WebAuthnAuth, U256, U256)>::decode(&params, 0)
                .map_err(|_| ExitCode::MalformedBuiltinParams)?;

        verify_webauthn(
            &challenge,
            require_user_verification,
            &auth,
            x,
            y,
            gas_limit,
        )?
    } else if selector == VERIFY_STRICT_SELECTOR {
        let (
            challenge,
            require_user_verification,
            expected_rp_id_hash,
            expected_origin,
            auth,
            x,
            y,
        ) = SolidityABI::<(Bytes, bool, B256, Bytes, WebAuthnAuth, U256, U256)>::decode(&params, 0)
            .map_err(|_| ExitCode::MalformedBuiltinParams)?;

        verify_webauthn_with_policy(
            &challenge,
            require_user_verification,
            &WebAuthnPolicy {
                expected_rp_id_hash,
                expected_origin,
            },
            &auth,
            x,
            y,
            gas_limit,
        )?
    } else {
        return Err(ExitCode::MalformedBuiltinParams);
    };

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

    type StrictCallParams = (Bytes, bool, B256, Bytes, WebAuthnAuth, U256, U256);

    fn valid_call_params(
        require_user_verification: bool,
    ) -> (Bytes, bool, WebAuthnAuth, U256, U256) {
        let challenge = Bytes::copy_from_slice(
            &hex::decode("f631058a3ba1116acce12396fad0a125b5041c43f8e15723709f81aa8d5f4ccf")
                .unwrap(),
        );
        let client_data_json = Bytes::copy_from_slice(
            format!(
                "{{\"type\":\"webauthn.get\",\"challenge\":\"{}\",\"origin\":\"http://localhost:3005\"}}",
                webauthn::base64url_encode(&challenge)
            )
            .as_bytes(),
        );

        signed_call_params(challenge, require_user_verification, client_data_json)
    }

    /// Builds call parameters with a fresh key pair signing the supplied client data, so the
    /// assertion is cryptographically valid and only the client data policy is under test.
    fn signed_call_params(
        challenge: Bytes,
        require_user_verification: bool,
        client_data_json: Bytes,
    ) -> (Bytes, bool, WebAuthnAuth, U256, U256) {
        let authenticator_data = Bytes::copy_from_slice(
            &hex::decode(
                "49960de5880e8c687434170f6476605b8fe4aeb9a28632c7995cf3ba831d97630500000101",
            )
            .unwrap(),
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

    fn encode_strict_call(params_tuple: &StrictCallParams) -> Vec<u8> {
        let mut params = BytesMut::new();
        SolidityABI::<(Bytes, bool, B256, Bytes, WebAuthnAuth, U256, U256)>::encode(
            params_tuple,
            &mut params,
            0,
        )
        .unwrap();

        let mut input = VERIFY_STRICT_SELECTOR.to_vec();
        input.extend_from_slice(&params);
        input
    }

    fn strict_call_params(params: (Bytes, bool, WebAuthnAuth, U256, U256)) -> StrictCallParams {
        let (challenge, require_user_verification, auth, x, y) = params;
        let expected_rp_id_hash = B256::from_slice(&auth.authenticator_data[..32]);
        let expected_origin = Bytes::copy_from_slice(b"http://localhost:3005");

        (
            challenge,
            require_user_verification,
            expected_rp_id_hash,
            expected_origin,
            auth,
            x,
            y,
        )
    }

    fn valid_strict_call_input(require_user_verification: bool) -> (Vec<u8>, StrictCallParams) {
        let params = strict_call_params(valid_call_params(require_user_verification));

        (encode_strict_call(&params), params)
    }

    /// Signs `client_data_json` and runs it through the strict entrypoint with a matching policy.
    fn strict_result_for_client_data(client_data_json: &str) -> Vec<u8> {
        let challenge = Bytes::copy_from_slice(
            &hex::decode("f631058a3ba1116acce12396fad0a125b5041c43f8e15723709f81aa8d5f4ccf")
                .unwrap(),
        );
        let params = strict_call_params(signed_call_params(
            challenge,
            true,
            Bytes::copy_from_slice(client_data_json.as_bytes()),
        ));

        let (output, _) = exec(&encode_strict_call(&params), WEBAUTHN_VERIFY_GAS).unwrap();
        output
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
    fn strict_valid_signature_returns_true_and_charges_fuel() {
        let (input, _) = valid_strict_call_input(true);
        let (output, fuel) = exec(&input, WEBAUTHN_VERIFY_GAS).unwrap();
        assert_eq!(output, B256::with_last_byte(1)[..]);
        assert_eq!(fuel, WEBAUTHN_VERIFY_GAS * FUEL_DENOM_RATE);
    }

    #[test]
    fn strict_mismatched_rp_id_hash_returns_false() {
        let (_, mut params) = valid_strict_call_input(true);
        params.2 = B256::with_last_byte(1);

        let (output, fuel) = exec(&encode_strict_call(&params), WEBAUTHN_VERIFY_GAS).unwrap();
        assert_eq!(output, B256::default()[..]);
        assert_eq!(fuel, WEBAUTHN_VERIFY_GAS * FUEL_DENOM_RATE);
    }

    #[test]
    fn strict_disallowed_origin_returns_false() {
        let (_, mut params) = valid_strict_call_input(true);
        params.3 = Bytes::copy_from_slice(b"https://example.com");

        let (output, fuel) = exec(&encode_strict_call(&params), WEBAUTHN_VERIFY_GAS).unwrap();
        assert_eq!(output, B256::default()[..]);
        assert_eq!(fuel, WEBAUTHN_VERIFY_GAS * FUEL_DENOM_RATE);
    }

    #[test]
    fn strict_accepts_conforming_client_data_variants() {
        let challenge = webauthn::base64url_encode(
            &hex::decode("f631058a3ba1116acce12396fad0a125b5041c43f8e15723709f81aa8d5f4ccf")
                .unwrap(),
        );
        let variants = [
            // Chrome-style object with the extra members conforming clients append.
            format!(
                "{{\"type\":\"webauthn.get\",\"challenge\":\"{challenge}\",\
                 \"origin\":\"http://localhost:3005\",\"crossOrigin\":false,\
                 \"other_keys_can_be_added_here\":\"do not compare clientDataJSON against a \
                 template. See https://goo.gl/yabPex\"}}"
            ),
            // Members in a different order, which the index-based check could not accept.
            format!(
                "{{\"origin\":\"http://localhost:3005\",\"challenge\":\"{challenge}\",\
                 \"type\":\"webauthn.get\"}}"
            ),
            // Escaped solidus in the origin, which decodes to the expected origin.
            format!(
                "{{\"type\":\"webauthn.get\",\"challenge\":\"{challenge}\",\
                 \"origin\":\"http:\\/\\/localhost:3005\"}}"
            ),
        ];

        for client_data_json in variants {
            assert_eq!(
                strict_result_for_client_data(&client_data_json),
                B256::with_last_byte(1)[..],
                "expected {client_data_json} to verify"
            );
        }
    }

    #[test]
    fn strict_rejects_ambiguous_or_malformed_client_data() {
        let challenge = webauthn::base64url_encode(
            &hex::decode("f631058a3ba1116acce12396fad0a125b5041c43f8e15723709f81aa8d5f4ccf")
                .unwrap(),
        );
        let rejected = [
            // Duplicate origin: the expected origin is present, but the object is ambiguous.
            format!(
                "{{\"type\":\"webauthn.get\",\"challenge\":\"{challenge}\",\
                 \"origin\":\"http://localhost:3005\",\"origin\":\"https://evil.example\"}}"
            ),
            // Duplicate type, where only the decoy is an assertion type.
            format!(
                "{{\"type\":\"webauthn.get\",\"type\":\"webauthn.create\",\
                 \"challenge\":\"{challenge}\",\"origin\":\"http://localhost:3005\"}}"
            ),
            // A decoy origin hidden in an unknown member's string value.
            format!(
                "{{\"type\":\"webauthn.get\",\"challenge\":\"{challenge}\",\
                 \"decoy\":\"\\\"origin\\\":\\\"http://localhost:3005\\\"\",\
                 \"origin\":\"https://evil.example\"}}"
            ),
            // Cross-origin assertion, which the single-origin policy does not allow.
            format!(
                "{{\"type\":\"webauthn.get\",\"challenge\":\"{challenge}\",\
                 \"origin\":\"http://localhost:3005\",\"crossOrigin\":true}}"
            ),
            // Padded challenge encoding instead of the canonical base64url form.
            format!(
                "{{\"type\":\"webauthn.get\",\"challenge\":\"{challenge}=\",\
                 \"origin\":\"http://localhost:3005\"}}"
            ),
            // Trailing object after the client data.
            format!(
                "{{\"type\":\"webauthn.get\",\"challenge\":\"{challenge}\",\
                 \"origin\":\"http://localhost:3005\"}}{{\"origin\":\"https://evil.example\"}}"
            ),
            // Missing origin altogether.
            format!("{{\"type\":\"webauthn.get\",\"challenge\":\"{challenge}\"}}"),
        ];

        for client_data_json in rejected {
            assert_eq!(
                strict_result_for_client_data(&client_data_json),
                B256::default()[..],
                "expected {client_data_json} to be rejected"
            );
        }
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
