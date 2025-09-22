/// Curve256r1 verify
use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use p256::{
    ecdsa::{Signature, VerifyingKey},
    elliptic_curve::FieldBytes,
    PublicKey,
};
use rwasm::{Store, TrapCode, TypedCaller, Value};

/// Gas cost for p256 verification (based on EIP-7212)
const P256_VERIFY_GAS: u64 = 3450;

/// Error type for p256 verification operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrecompileError {
    /// Invalid input length
    InvalidInputLength,
    /// Invalid signature
    InvalidSignature,
    /// Invalid public key
    InvalidPublicKey,
    /// Verification failed
    VerificationFailed,
}

pub struct SyscallCurve256r1Verify {}

impl SyscallCurve256r1Verify {
    pub const fn new() -> Self {
        Self {}
    }
}

/// Reconstructs a p256 signature from r and s components
fn reconstruct_signature(r_bytes: &[u8], s_bytes: &[u8]) -> Result<Signature, ()> {
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
fn reconstruct_public_key(x_bytes: &[u8], y_bytes: &[u8]) -> Result<VerifyingKey, ()> {
    // Create uncompressed public key format (0x04 prefix + x + y)
    let mut uncompressed_key = Vec::with_capacity(65);
    uncompressed_key.push(0x04); // Uncompressed format marker
    uncompressed_key.extend_from_slice(x_bytes);
    uncompressed_key.extend_from_slice(y_bytes);

    // Parse as public key
    let public_key = PublicKey::from_sec1_bytes(&uncompressed_key).map_err(|_| ())?;
    Ok(VerifyingKey::from(&public_key))
}

impl SyscallCurve256r1Verify {
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let input_ptr = params[0].i32().unwrap() as usize;
        let input_len = params[1].i32().unwrap() as usize;
        let output_ptr = params[2].i32().unwrap() as usize;

        // Read input from memory
        let mut input = vec![0u8; input_len];
        caller.memory_read(input_ptr, &mut input)?;

        // Perform verification
        let verification_result = Self::fn_impl(&input);

        // Write result to memory
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
        // Check input length (160 bytes: 32 + 32 + 32 + 32 + 32)
        if input.len() != 160 {
            return false;
        }

        // Parse input components
        let message_hash = &input[0..32];
        let r_bytes = &input[32..64];
        let s_bytes = &input[64..96];
        let pubkey_x = &input[96..128];
        let pubkey_y = &input[128..160];

        // Reconstruct the signature from r and s components
        let signature = match reconstruct_signature(r_bytes, s_bytes) {
            Ok(sig) => sig,
            Err(_) => {
                #[cfg(test)]
                println!("Failed to reconstruct signature from r and s");
                return false;
            }
        };

        // Reconstruct the public key from x and y coordinates
        let verifying_key = match reconstruct_public_key(pubkey_x, pubkey_y) {
            Ok(key) => key,
            Err(_) => {
                #[cfg(test)]
                println!("Failed to reconstruct public key from x and y");
                return false;
            }
        };

        // Verify the signature against the provided 32-byte prehash directly (no rehashing)
        use p256::ecdsa::signature::hazmat::PrehashVerifier;
        verifying_key
            .verify_prehash(message_hash, &signature)
            .is_ok()
    }
}

impl From<PrecompileError> for ExitCode {
    fn from(err: PrecompileError) -> Self {
        match err {
            PrecompileError::InvalidInputLength => ExitCode::InputOutputOutOfBounds,
            PrecompileError::InvalidSignature => ExitCode::PrecompileError,
            PrecompileError::InvalidPublicKey => ExitCode::PrecompileError,
            PrecompileError::VerificationFailed => ExitCode::PrecompileError,
        }
    }
}
