#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;

use alloc::vec;
use fluentbase_sdk::{alloc_slice, entrypoint, Bytes, ContextReader, ExitCode, SharedAPI};

const INPUT_LENGTH: usize = 160;
const P256_VERIFY_GAS: u64 = 3450;

/// Helper function for common validation and gas checking pattern
#[inline(always)]
fn validate_and_consume_gas<SDK: SharedAPI>(
    sdk: &mut SDK,
    gas_cost: u64,
    gas_limit: u64,
    input: &[u8],
) -> bool {
    if !verify_input_length(&input) {
        sdk.sync_evm_gas(gas_cost, 0);
        sdk.write(&[]);
        return false;
    }
    if gas_cost > gas_limit {
        sdk.native_exit(ExitCode::OutOfFuel);
    }
    sdk.sync_evm_gas(gas_cost, 0);
    true
}

#[inline(always)]
fn verify_input_length(input: &[u8]) -> bool {
    if input.len() == INPUT_LENGTH {
        return true;
    }
    false
}

/// Main entry point for the secp256r1 wrapper contract.
/// This contract wraps the secp256r1 precompile (EIP-7212) which verifies ECDSA signatures
/// using the secp256r1 (P-256) elliptic curve.
///
/// Input format:
/// | signed message hash |  r  |  s  | public key x | public key y |
/// | :-----------------: | :-: | :-: | :----------: | :----------: |
/// |          32         | 32  | 32  |     32       |      32      |
///
/// Output:
/// - Returns a single byte with value 1 if the signature is valid
/// - Returns an empty byte array if the signature is invalid
pub fn main_entry<SDK: SharedAPI>(mut sdk: SDK) {
    let gas_limit = sdk.context().contract_gas_limit();
    let input_length = sdk.input_size();
    let mut input = alloc_slice(input_length as usize);
    sdk.read(&mut input, 0);
    let input = Bytes::copy_from_slice(input);

    if !validate_and_consume_gas(&mut sdk, P256_VERIFY_GAS, gas_limit, &input) {
        return;
    }

    unimplemented!()
    // let verification_result = SDK::curve256r1_verify(&input);
    // if verification_result {
    //     let mut result = vec![0u8; 32];
    //     result[31] = 1;
    //     sdk.write(&result);
    // } else {
    //     sdk.write(&[]);
    // }
}

entrypoint!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{hex, ContractContextV1, B256, FUEL_DENOM_RATE};
    use fluentbase_testing::HostTestingContext;
    use p256::{
        ecdsa::{signature::Verifier, SigningKey, VerifyingKey},
        elliptic_curve::rand_core::OsRng,
    };

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
    fn test_valid_signature() {
        // Test vector from secp256r1.rs tests
        exec_evm_precompile(
            &hex!("4cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4da73bd4903f0ce3b639bbbf6e8e80d16931ff4bcf5993d58468e8fb19086e8cac36dbcd03009df8c59286b162af3bd7fcc0450c9aa81be5d10d312af6c66b1d604aebd3099c618202fcfe16ae7770b0c49ab5eadf74b754204a3bb6060e44eff37618b065f9832de4ca6ca971a7a1adc826d0f7c00181a5fb2ddf79ae00b4e10e"),
            &&B256::with_last_byte(1)[..],
            3450,
        );
    }
    #[test]
    fn test_valid_signature_custom() {
        // Test vector from secp256r1.rs tests
        exec_evm_precompile(
            &hex!("e775723953ead4a90411a02908fd1a629db584bc600664c609061f221ef6bf7cbe0aca61c884167420c8d16b6a22b5952ab46586c0fdba026cd0bf32258c500434091e9ec6503491ed0a820c38ad82c672d88c9fe8e681accdf7e26dbd2d7958b6114ed3a5b9bf7255eda3077b2c63ad476d481d979699a5d22bd030077dab338bea1ad18733e7d410649a8b09c6429ea065c9d66aaf6a8e793b0567eadd942d"),
            &&B256::with_last_byte(1)[..],
            3450,
        );
    }

    #[test]
    fn test_invalid_signature() {
        // Modified test vector with invalid signature
        exec_evm_precompile(
            &hex!("4cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4dffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff4aebd3099c618202fcfe16ae7770b0c49ab5eadf74b754204a3bb6060e44eff37618b065f9832de4ca6ca971a7a1adc826d0f7c00181a5fb2ddf79ae00b4e10e"),
            &hex!(""),
            3450,
        );
    }

    #[test]
    fn test_invalid_public_key() {
        // Modified test vector with invalid public key
        exec_evm_precompile(
            &hex!("4cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4da73bd4903f0ce3b639bbbf6e8e80d16931ff4bcf5993d58468e8fb19086e8cac36dbcd03009df8c59286b162af3bd7fcc0450c9aa81be5d10d312af6c66b1d6000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
            &hex!(""),
            3450,
        );
    }

    #[test]
    fn test_incorrect_input_length() {
        // Test with too short input
        exec_evm_precompile(&hex!("4cee90eb86eaa050036147a12d49004b6a"), &hex!(""), 3450);
    }

    #[test]
    fn test_p256_signature_format() -> Result<(), Box<dyn std::error::Error>> {
        let (signing_key, verifying_key) = generate_key_pair();
        let (x, y) = extract_public_key_coordinates(&verifying_key);

        let test_message = "deadbeef0001";
        let message_hash = hash_message(test_message)?;

        let signature = sign_message(&signing_key, test_message)?;
        let (r, s) = extract_signature_components(&signature);

        let precompiled_input = build_precompile_input(&message_hash, &r, &s, &x, &y);
        let verify_result = verifying_key.verify(hex::decode(test_message)?.as_slice(), &signature);

        // Log results
        print_test_results(
            test_message,
            &message_hash,
            &x,
            &y,
            &precompiled_input,
            verify_result.is_ok(),
        );

        Ok(())
    }

    /// Generates a P-256 key pair
    fn generate_key_pair() -> (SigningKey, VerifyingKey) {
        let signing_key = SigningKey::random(&mut OsRng);
        let verifying_key = VerifyingKey::from(&signing_key);

        (signing_key, verifying_key)
    }

    /// Extracts x and y coordinates from a verifying key
    fn extract_public_key_coordinates(verifying_key: &VerifyingKey) -> (Vec<u8>, Vec<u8>) {
        let encoded_point = verifying_key.to_encoded_point(false);
        let uncompressed_bytes = encoded_point.as_bytes();

        // Verify format (should start with 0x04 followed by 64 bytes)
        assert_eq!(
            uncompressed_bytes[0], 0x04,
            "Expected uncompressed format marker"
        );
        assert_eq!(
            uncompressed_bytes.len(),
            65,
            "Expected 65 bytes (1 + 32 + 32)"
        );

        // Extract the x and y coordinates (32 bytes each)
        let x = uncompressed_bytes[1..33].to_vec();
        let y = uncompressed_bytes[33..65].to_vec();

        (x, y)
    }

    /// Hashes a hex-encoded message using SHA-256
    fn hash_message(message_hex: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        use hex;
        use sha2::{Digest, Sha256};

        let message = hex::decode(message_hex)?;
        let mut hasher = Sha256::new();
        hasher.update(&message);
        let hash = hasher.finalize().to_vec();

        Ok(hash)
    }

    /// Signs a hex-encoded message
    fn sign_message(
        signing_key: &SigningKey,
        message_hex: &str,
    ) -> Result<p256::ecdsa::Signature, Box<dyn std::error::Error>> {
        use hex;
        use p256::ecdsa::signature::Signer;

        let message = hex::decode(message_hex)?;
        let signature = signing_key.sign(&message);

        Ok(signature)
    }

    /// Extracts r and s values from a signature
    fn extract_signature_components(signature: &p256::ecdsa::Signature) -> (Vec<u8>, Vec<u8>) {
        let signature_bytes = signature.to_bytes();

        // Check signature format - should be exactly 64 bytes (32 for r, 32 for s)
        assert_eq!(signature_bytes.len(), 64, "Expected 64-byte signature");

        let r = signature_bytes[0..32].to_vec();
        let s = signature_bytes[32..64].to_vec();

        (r, s)
    }

    /// Builds precompiled input for the verification
    fn build_precompile_input(
        msg_hash: &[u8],
        r: &[u8],
        s: &[u8],
        public_key_x: &[u8],
        public_key_y: &[u8],
    ) -> Vec<u8> {
        let mut precompile_input = Vec::with_capacity(32 + 32 + 32 + 32 + 32);
        precompile_input.extend_from_slice(msg_hash); // 32 bytes message hash
        precompile_input.extend_from_slice(r); // 32 bytes r value
        precompile_input.extend_from_slice(s); // 32 bytes s value
        precompile_input.extend_from_slice(public_key_x); // 32 bytes public key x
        precompile_input.extend_from_slice(public_key_y); // 32 bytes public key y

        precompile_input
    }

    /// Prints test results and important values
    fn print_test_results(
        message_hex: &str,
        message_hash: &[u8],
        public_key_x: &[u8],
        public_key_y: &[u8],
        precompiled_input: &[u8],
        verification_success: bool,
    ) {
        println!("Public Key x: {}", hex::encode(public_key_x));
        println!("Public Key y: {}", hex::encode(public_key_y));
        println!("Message: {}", message_hex);
        println!("Message hash: {}", hex::encode(message_hash));
        println!("Precompiled input: {}", hex::encode(precompiled_input));
        println!("Local verification result: {}", verification_success);
    }
}
