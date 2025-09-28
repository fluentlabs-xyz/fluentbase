use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{Store, TrapCode, TypedCaller, Value};
use sp1_curves::CurveType;
use std::marker::PhantomData;

use super::config::VerifyConfig;

/// Error type for verification operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyError {
    /// Invalid input length
    InvalidInputLength,
    /// Invalid signature format
    InvalidSignature,
    /// Invalid public key format
    InvalidPublicKey,
    /// Verification failed
    VerificationFailed,
    /// Unsupported curve type
    UnsupportedCurve,
}

/// Parsed input components for signature verification
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyInputComponents<'a> {
    pub message_hash: &'a [u8],
    pub r_bytes: &'a [u8],
    pub s_bytes: &'a [u8],
    pub pubkey_x: &'a [u8],
    pub pubkey_y: &'a [u8],
}

impl From<VerifyError> for ExitCode {
    fn from(err: VerifyError) -> Self {
        match err {
            VerifyError::InvalidInputLength => ExitCode::InputOutputOutOfBounds,
            VerifyError::InvalidSignature => ExitCode::PrecompileError,
            VerifyError::InvalidPublicKey => ExitCode::PrecompileError,
            VerifyError::VerificationFailed => ExitCode::PrecompileError,
            VerifyError::UnsupportedCurve => ExitCode::PrecompileError,
        }
    }
}

/// Generic Weierstrass verify syscall handler
///
/// This handler provides a generic interface for signature verification operations
/// on different Weierstrass curves. It dispatches to curve-specific implementations
/// based on the provided configuration.
pub struct SyscallWeierstrassVerifyAssign<C: VerifyConfig> {
    _phantom: PhantomData<C>,
}

impl<C: VerifyConfig> SyscallWeierstrassVerifyAssign<C> {
    pub const fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }

    /// Parse input components for signature verification
    ///
    /// This function extracts the message hash, signature components (r, s),
    /// and public key coordinates (x, y) from the input bytes according to
    /// the configuration's layout specification.
    fn parse_input_components(input: &[u8]) -> Result<VerifyInputComponents, VerifyError> {
        // Check input length
        if input.len() != C::TOTAL_INPUT_SIZE {
            return Err(VerifyError::InvalidInputLength);
        }

        // Parse input components according to the layout:
        // [message_hash][r][s][pubkey_x][pubkey_y]
        let message_hash = &input[0..C::MESSAGE_HASH_SIZE];
        let r_bytes = &input[C::MESSAGE_HASH_SIZE..C::MESSAGE_HASH_SIZE + C::SIGNATURE_R_SIZE];
        let s_bytes = &input[C::MESSAGE_HASH_SIZE + C::SIGNATURE_R_SIZE
            ..C::MESSAGE_HASH_SIZE + C::SIGNATURE_R_SIZE + C::SIGNATURE_S_SIZE];
        let pubkey_x = &input[C::MESSAGE_HASH_SIZE + C::SIGNATURE_R_SIZE + C::SIGNATURE_S_SIZE
            ..C::MESSAGE_HASH_SIZE
                + C::SIGNATURE_R_SIZE
                + C::SIGNATURE_S_SIZE
                + C::PUBLIC_KEY_X_SIZE];
        let pubkey_y = &input[C::MESSAGE_HASH_SIZE
            + C::SIGNATURE_R_SIZE
            + C::SIGNATURE_S_SIZE
            + C::PUBLIC_KEY_X_SIZE..C::TOTAL_INPUT_SIZE];

        Ok(VerifyInputComponents {
            message_hash,
            r_bytes,
            s_bytes,
            pubkey_x,
            pubkey_y,
        })
    }

    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let input_ptr = params[0].i32().unwrap() as usize;
        let input_len = params[1].i32().unwrap() as usize;
        let output_ptr = params[2].i32().unwrap() as usize;

        let mut input = vec![0u8; input_len];
        caller.memory_read(input_ptr, &mut input)?;

        let verification_result = Self::fn_impl(&input);

        let result_bytes = if verification_result {
            let mut success_result = vec![0u8; 32];
            success_result[31] = 1; // success marker
            success_result
        } else {
            vec![] // empty result for failure
        };

        caller.memory_write(output_ptr, &result_bytes)?;
        result[0] = Value::I32(verification_result as i32);

        Ok(())
    }

    pub fn fn_impl(input: &[u8]) -> bool {
        // Dispatch based on curve type
        match C::CURVE_TYPE {
            CurveType::Secp256r1 => Self::secp256r1_verify_impl(input),
            CurveType::Secp256k1 => Self::secp256k1_verify_impl(input),
            _ => false, // Unsupported curve type
        }
    }

    /// Secp256r1 verification implementation
    fn secp256r1_verify_impl(input: &[u8]) -> bool {
        let components = match Self::parse_input_components(input) {
            Ok(components) => components,
            Err(_) => return false,
        };

        // Reconstruct the signature from r and s components
        let signature =
            match Self::reconstruct_p256_signature(components.r_bytes, components.s_bytes) {
                Ok(sig) => sig,
                Err(_) => return false,
            };

        // Reconstruct the public key from x and y coordinates
        let verifying_key =
            match Self::reconstruct_p256_public_key(components.pubkey_x, components.pubkey_y) {
                Ok(key) => key,
                Err(_) => return false,
            };

        // Verify the signature against the provided 32-byte prehash directly (no rehashing)
        use p256::ecdsa::signature::hazmat::PrehashVerifier;
        verifying_key
            .verify_prehash(components.message_hash, &signature)
            .is_ok()
    }

    /// Secp256k1 verification implementation
    fn secp256k1_verify_impl(input: &[u8]) -> bool {
        let components = match Self::parse_input_components(input) {
            Ok(components) => components,
            Err(_) => return false,
        };

        // Reconstruct the signature from r and s components
        let signature =
            match Self::reconstruct_secp256k1_signature(components.r_bytes, components.s_bytes) {
                Ok(sig) => sig,
                Err(_) => return false,
            };

        // Reconstruct the public key from x and y coordinates
        let verifying_key = match Self::reconstruct_secp256k1_public_key(
            components.pubkey_x,
            components.pubkey_y,
        ) {
            Ok(key) => key,
            Err(_) => return false,
        };

        // Verify the signature against the provided 32-byte prehash directly (no rehashing)
        use secp256k1::{Message, SECP256K1};
        let msg = match Message::from_digest_slice(components.message_hash) {
            Ok(msg) => msg,
            Err(_) => return false,
        };
        SECP256K1
            .verify_ecdsa(&msg, &signature, &verifying_key)
            .is_ok()
    }

    /// Reconstructs a p256 signature from r and s components
    fn reconstruct_p256_signature(
        r_bytes: &[u8],
        s_bytes: &[u8],
    ) -> Result<p256::ecdsa::Signature, ()> {
        use p256::{ecdsa::Signature, elliptic_curve::FieldBytes};

        let r_field = FieldBytes::<p256::NistP256>::from_slice(r_bytes);
        let s_field = FieldBytes::<p256::NistP256>::from_slice(s_bytes);

        if let Ok(sig) = Signature::from_scalars(*r_field, *s_field) {
            return Ok(sig);
        }

        let mut sig_bytes = [0u8; 64];
        sig_bytes[0..32].copy_from_slice(r_bytes);
        sig_bytes[32..64].copy_from_slice(s_bytes);

        Signature::from_bytes(&sig_bytes.into()).map_err(|_| ())
    }

    /// Reconstructs a p256 public key from x and y coordinates
    fn reconstruct_p256_public_key(
        x_bytes: &[u8],
        y_bytes: &[u8],
    ) -> Result<p256::ecdsa::VerifyingKey, ()> {
        use p256::{ecdsa::VerifyingKey, PublicKey};

        // Create uncompressed public key format (0x04 prefix + x + y)
        let mut uncompressed_key = Vec::with_capacity(65);
        uncompressed_key.push(0x04); // Uncompressed format marker
        uncompressed_key.extend_from_slice(x_bytes);
        uncompressed_key.extend_from_slice(y_bytes);

        // Parse as public key
        let public_key = PublicKey::from_sec1_bytes(&uncompressed_key).map_err(|_| ())?;
        Ok(VerifyingKey::from(&public_key))
    }

    /// Reconstructs a secp256k1 signature from r and s components
    fn reconstruct_secp256k1_signature(
        r_bytes: &[u8],
        s_bytes: &[u8],
    ) -> Result<secp256k1::ecdsa::Signature, ()> {
        use secp256k1::ecdsa::Signature;

        let mut sig_bytes = [0u8; 64];
        sig_bytes[0..32].copy_from_slice(r_bytes);
        sig_bytes[32..64].copy_from_slice(s_bytes);

        Signature::from_compact(&sig_bytes).map_err(|_| ())
    }

    /// Reconstructs a secp256k1 public key from x and y coordinates
    fn reconstruct_secp256k1_public_key(
        x_bytes: &[u8],
        y_bytes: &[u8],
    ) -> Result<secp256k1::PublicKey, ()> {
        use secp256k1::PublicKey;

        // Create uncompressed public key format (0x04 prefix + x + y)
        let mut uncompressed_key = Vec::with_capacity(65);
        uncompressed_key.push(0x04); // Uncompressed format marker
        uncompressed_key.extend_from_slice(x_bytes);
        uncompressed_key.extend_from_slice(y_bytes);

        // Parse as public key
        PublicKey::from_slice(&uncompressed_key).map_err(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::super::config::{Secp256k1VerifyConfig, Secp256r1VerifyConfig};
    use super::*;
    use hex_literal::hex;

    // Test vectors for secp256r1 verification
    const SECP256R1_TEST_VECTOR: &[u8] = &hex!(
        "0000000000000000000000000000000000000000000000000000000000000000" // message hash
        "0000000000000000000000000000000000000000000000000000000000000000" // r
        "0000000000000000000000000000000000000000000000000000000000000000" // s
        "0000000000000000000000000000000000000000000000000000000000000000" // pubkey x
        "0000000000000000000000000000000000000000000000000000000000000000" // pubkey y
    );

    #[test]
    fn test_secp256r1_verify_config() {
        assert_eq!(Secp256r1VerifyConfig::CURVE_TYPE, CurveType::Secp256r1);
        assert_eq!(Secp256r1VerifyConfig::MESSAGE_HASH_SIZE, 32);
        assert_eq!(Secp256r1VerifyConfig::SIGNATURE_R_SIZE, 32);
        assert_eq!(Secp256r1VerifyConfig::SIGNATURE_S_SIZE, 32);
        assert_eq!(Secp256r1VerifyConfig::PUBLIC_KEY_X_SIZE, 32);
        assert_eq!(Secp256r1VerifyConfig::PUBLIC_KEY_Y_SIZE, 32);
        assert_eq!(Secp256r1VerifyConfig::TOTAL_INPUT_SIZE, 160);
    }

    #[test]
    fn test_secp256k1_verify_config() {
        assert_eq!(Secp256k1VerifyConfig::CURVE_TYPE, CurveType::Secp256k1);
        assert_eq!(Secp256k1VerifyConfig::MESSAGE_HASH_SIZE, 32);
        assert_eq!(Secp256k1VerifyConfig::SIGNATURE_R_SIZE, 32);
        assert_eq!(Secp256k1VerifyConfig::SIGNATURE_S_SIZE, 32);
        assert_eq!(Secp256k1VerifyConfig::PUBLIC_KEY_X_SIZE, 32);
        assert_eq!(Secp256k1VerifyConfig::PUBLIC_KEY_Y_SIZE, 32);
        assert_eq!(Secp256k1VerifyConfig::TOTAL_INPUT_SIZE, 160);
    }

    #[test]
    fn test_invalid_input_length() {
        let short_input = [0u8; 32]; // Too short
        assert!(!SyscallWeierstrassVerifyAssign::<Secp256r1VerifyConfig>::fn_impl(&short_input));
        assert!(!SyscallWeierstrassVerifyAssign::<Secp256k1VerifyConfig>::fn_impl(&short_input));
    }

    #[test]
    fn test_verify_error_conversion() {
        let exit_code: ExitCode = VerifyError::InvalidInputLength.into();
        assert_eq!(exit_code, ExitCode::InputOutputOutOfBounds);

        let exit_code: ExitCode = VerifyError::InvalidSignature.into();
        assert_eq!(exit_code, ExitCode::PrecompileError);

        let exit_code: ExitCode = VerifyError::VerificationFailed.into();
        assert_eq!(exit_code, ExitCode::PrecompileError);
    }
}
