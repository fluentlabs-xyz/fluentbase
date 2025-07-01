#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
mod webauthn;

use fluentbase_sdk::{
    alloc_slice,
    codec::SolidityABI,
    entrypoint,
    Bytes,
    ContextReader,
    ExitCode,
    SharedAPI,
    B256,
    U256,
};
use webauthn::{verify_webauthn, WebAuthnAuth};

/// Function selector: 0x94516dde
/// Derived from:
/// keccak256("verify(bytes,bool,(bytes,bytes,uint256,uint256,bytes32,bytes32),uint256,uint256)")

const VERIFY_SELECTOR: [u8; 4] = [0x94, 0x51, 0x6d, 0xde];

/// WebAuthn verification contract for blockchain authentication
///
/// Based on reference implementations:
/// - Solady: https://github.com/vectorized/solady/blob/main/src/utils/WebAuthn.sol
/// - Daimo: https://github.com/daimo-eth/p256-verifier/blob/master/src/WebAuthn.sol
/// - Coinbase: https://github.com/base-org/webauthn-sol/blob/main/src/WebAuthn.sol

pub fn main_entry(mut sdk: impl SharedAPI) {
    // Read input
    let input_length = sdk.input_size();
    assert!(
        input_length >= 4,
        "webauthn: input should be at least 4 bytes"
    );

    let mut input = alloc_slice(input_length as usize);
    sdk.read(&mut input, 0);

    let (selector, params) = input.split_at(4);
    if selector != VERIFY_SELECTOR {
        panic!("webauthn: invalid function selector; expected 0xbccf2ab7");
    }

    // Decode WebAuthn parameters using Solidity ABI
    let (challenge, require_user_verification, auth, x, y) =
        SolidityABI::<(Bytes, bool, WebAuthnAuth, U256, U256)>::decode(&Bytes::from(params), 0)
            .unwrap_or_else(|_| panic!("webauthn: failed to decode input parameters"));

    // Get gas limit for precompile call
    let gas_limit = sdk.context().contract_gas_limit();

    let result = match verify_webauthn(
        &challenge,
        require_user_verification,
        &auth,
        x,
        y,
        gas_limit,
    ) {
        Ok(is_valid) => {
            if is_valid {
                // Return 32-byte output with last byte set to 1 if verification succeeds
                B256::with_last_byte(1)
            } else {
                // Return empty output if verification fails
                B256::default()
            }
        }
        Err(err) => sdk.native_exit(ExitCode::from(err)),
    };

    // Write the result
    sdk.write(&result[..]);
}

entrypoint!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{Bytes, ContractContextV1, B256};
    use fluentbase_sdk_testing::HostTestingContext;

    fn assert_call_eq(input: &[u8], expected: &[u8]) {
        let gas_limit = 100_000;
        let sdk = HostTestingContext::default()
            .with_input(Bytes::copy_from_slice(input))
            .with_contract_context(ContractContextV1 {
                gas_limit,
                ..Default::default()
            });

        main_entry(sdk.clone());

        let output = sdk.take_output();
        assert_eq!(output.len(), expected.len(), "Output length mismatch");
        println!("Output: {:?}", hex::encode(&output));

        assert_eq!(output, expected, "Verification result mismatch");
    }

    #[test]
    fn test_valid_signature() {
        // Create test parameters with valid WebAuthn data
        let input = hex::decode("94516dde000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000a0000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000e0e867625216bc4bbd847780a7647c1e1cf1c6b036f1cc917a189f85789914329ee7eaf13acb291279c9a974a1f12209924f61e9731d74f56bdb0d0b49ae3658b80000000000000000000000000000000000000000000000000000000000000020f631058a3ba1116acce12396fad0a125b5041c43f8e15723709f81aa8d5f4ccf00000000000000000000000000000000000000000000000000000000000000c0000000000000000000000000000000000000000000000000000000000000012000000000000000000000000000000000000000000000000000000000000000170000000000000000000000000000000000000000000000000000000000000001041d2ba6320443e34a94700c83f8a65a49c647a679cd481b4756445e7994d3a26b9513bd0a2bf00fbf11973f7cd435fb62d529aeeb78243f84889deb6e4b5995000000000000000000000000000000000000000000000000000000000000002549960de5880e8c687434170f6476605b8fe4aeb9a28632c7995cf3ba831d9763050000010100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000727b2274797065223a22776562617574686e2e676574222c226368616c6c656e6765223a22396a4546696a75684557724d34534f572d7443684a625545484550343456636a634a2d42716f3166544d38222c226f726967696e223a22687474703a2f2f6c6f63616c686f73743a33303035227d0000000000000000000000000000").expect("Failed to decode webauthn params from hex");

        println!("Input: {:?}", hex::encode(&input));

        // Execute precompile with valid input
        assert_call_eq(&input, &B256::with_last_byte(1)[..]);
    }

    #[test]
    fn test_invalid_signature() {
        // Create test parameters with invalid WebAuthn data
        let input = hex::decode("94516dde000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000a0000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000e02c15fa3dcecbd28795321d8e14f30c0a09cb0a5fbb02b6860c264514502672ba513136f66506523bd369730ef1274b0e95f2d03b5bf26808921b5d97fff8f7de0000000000000000000000000000000000000000000000000000000000000020f631058a3ba1116acce12396fad0a125b5041c43f8e15723709f81aa8d5f4ccf00000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000001200000000000000000000000000000000000000000000000000000000000000017000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002549960de5880e8c687434170f6476605b8fe4aeb9a28632c7995cf3ba831d9763050000010100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000727b2274797065223a22776562617574686e2e676574222c226368616c6c656e6765223a22396a4546696a75684557724d34534f572d7443684a625545484550343456636a634a2d42716f3166544d38222c226f726967696e223a22687474703a2f2f6c6f63616c686f73743a33303035227d0000000000000000000000000000").unwrap();

        assert_call_eq(&input, &B256::default()[..]);
    }
}
