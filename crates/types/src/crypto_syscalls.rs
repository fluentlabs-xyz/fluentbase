//! Precompile-level crypto syscalls.
//!
//! One syscall per operation of revm-precompile's `Crypto` trait: a whole pairing check, MSM,
//! signature recovery or KZG verification crosses to the host in a single call. The precompile
//! guests keep the EIP gas schedule, input parsing and error semantics; only the arithmetic moves.
//! The same operation list is what SP1 (patched crates) and OpenVM (`openvm-pairing`, k256, p256,
//! kzg) accelerate, so a zkVM host implements the same family behind the same ABI.
//!
//! # ABI
//!
//! Every syscall is `(input_ptr: i32, input_len: i32, output_ptr: i32) -> i32`. The payload is
//! the operation's operands in the `Crypto` trait encoding (unpadded big-endian field elements,
//! see [`CryptoSyscall::input_layout`]), the output is the fixed-size result written to
//! `output_ptr` on success, and the return value is `0` or a [`CryptoSyscallError`] code. Nothing
//! is written on error.
//!
//! # Fuel
//!
//! Engine-metered callers (contracts) are charged the EIP gas of the input by the handler.
//! Self-metered system runtimes (`RuntimeContext::engine_metered == false`) are not charged: the
//! precompile guest already charges the same gas through `sync_evm_gas`.

use strum_macros::{Display, FromRepr};

/// Unpadded BLS12-381 field element.
pub const CRYPTO_BLS_FP_LEN: usize = 48;
/// Unpadded BLS12-381 G1 point (`x || y`).
pub const CRYPTO_BLS_G1_LEN: usize = 2 * CRYPTO_BLS_FP_LEN;
/// Unpadded BLS12-381 G2 point (`x0 || x1 || y0 || y1`).
pub const CRYPTO_BLS_G2_LEN: usize = 4 * CRYPTO_BLS_FP_LEN;
/// BLS12-381 scalar.
pub const CRYPTO_BLS_SCALAR_LEN: usize = 32;
/// bn254 G1 point (`x || y`, 32-byte big-endian coordinates).
pub const CRYPTO_BN254_G1_LEN: usize = 64;
/// bn254 G2 point (128 bytes, EIP-197 order).
pub const CRYPTO_BN254_G2_LEN: usize = 128;
/// bn254 scalar.
pub const CRYPTO_BN254_SCALAR_LEN: usize = 32;
/// Largest payload a crypto syscall accepts (bounds the host-side copy).
pub const CRYPTO_SYSCALL_MAX_INPUT_LEN: usize = 4 * 1024 * 1024;

/// The precompile-level crypto operations, in `SysFuncIdx` order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Display, FromRepr)]
#[repr(u32)]
pub enum CryptoSyscall {
    /// `a(96) || b(96)` -> G1 point (96).
    Bls12381G1Add = 0x0901,
    /// `k * (point(96) || scalar(32))` -> G1 point (96).
    Bls12381G1Msm = 0x0902,
    /// `a(192) || b(192)` -> G2 point (192).
    Bls12381G2Add = 0x0903,
    /// `k * (point(192) || scalar(32))` -> G2 point (192).
    Bls12381G2Msm = 0x0904,
    /// `k * (g1(96) || g2(192))` -> one byte, `1` if the product of pairings is the identity.
    Bls12381PairingCheck = 0x0905,
    /// `fp(48)` -> G1 point (96).
    Bls12381MapFpToG1 = 0x0906,
    /// `fp2(96)` -> G2 point (192).
    Bls12381MapFp2ToG2 = 0x0907,
    /// `p1(64) || p2(64)` -> G1 point (64).
    Bn254G1Add = 0x0908,
    /// `point(64) || scalar(32)` -> G1 point (64).
    Bn254G1Mul = 0x0909,
    /// `k * (g1(64) || g2(128))` -> one byte, `1` if the product of pairings is the identity.
    Bn254PairingCheck = 0x090a,
    /// `sig(64) || recid(1) || msg(32)` -> 32-byte address-bearing hash.
    Secp256k1Ecrecover = 0x090b,
    /// `msg(32) || sig(64) || pk(64)` -> one byte, `1` if the signature verifies.
    Secp256r1Verify = 0x090c,
    /// `z(32) || y(32) || commitment(48) || proof(48)` -> one byte, `1` if the proof verifies.
    KzgVerifyProof = 0x090d,
}

impl CryptoSyscall {
    /// Fixed-size unit of a repeated payload, if the operation takes a list of operands.
    pub const fn unit_len(self) -> Option<usize> {
        match self {
            Self::Bls12381G1Msm => Some(CRYPTO_BLS_G1_LEN + CRYPTO_BLS_SCALAR_LEN),
            Self::Bls12381G2Msm => Some(CRYPTO_BLS_G2_LEN + CRYPTO_BLS_SCALAR_LEN),
            Self::Bls12381PairingCheck => Some(CRYPTO_BLS_G1_LEN + CRYPTO_BLS_G2_LEN),
            Self::Bn254PairingCheck => Some(CRYPTO_BN254_G1_LEN + CRYPTO_BN254_G2_LEN),
            _ => None,
        }
    }

    /// Exact payload length of a fixed-size operation.
    pub const fn fixed_input_len(self) -> Option<usize> {
        match self {
            Self::Bls12381G1Add => Some(2 * CRYPTO_BLS_G1_LEN),
            Self::Bls12381G2Add => Some(2 * CRYPTO_BLS_G2_LEN),
            Self::Bls12381MapFpToG1 => Some(CRYPTO_BLS_FP_LEN),
            Self::Bls12381MapFp2ToG2 => Some(2 * CRYPTO_BLS_FP_LEN),
            Self::Bn254G1Add => Some(2 * CRYPTO_BN254_G1_LEN),
            Self::Bn254G1Mul => Some(CRYPTO_BN254_G1_LEN + CRYPTO_BN254_SCALAR_LEN),
            Self::Secp256k1Ecrecover => Some(64 + 1 + 32),
            Self::Secp256r1Verify => Some(32 + 64 + 64),
            Self::KzgVerifyProof => Some(32 + 32 + 48 + 48),
            _ => None,
        }
    }

    /// Whether `len` is an acceptable payload length for this operation.
    pub const fn accepts_input_len(self, len: usize) -> bool {
        if len > CRYPTO_SYSCALL_MAX_INPUT_LEN {
            return false;
        }
        match (self.fixed_input_len(), self.unit_len()) {
            (Some(fixed), _) => len == fixed,
            (None, Some(unit)) => len.is_multiple_of(unit),
            (None, None) => false,
        }
    }

    /// Size of the result written to `output_ptr` on success.
    pub const fn output_len(self) -> usize {
        match self {
            Self::Bls12381G1Add | Self::Bls12381G1Msm | Self::Bls12381MapFpToG1 => {
                CRYPTO_BLS_G1_LEN
            }
            Self::Bls12381G2Add | Self::Bls12381G2Msm | Self::Bls12381MapFp2ToG2 => {
                CRYPTO_BLS_G2_LEN
            }
            Self::Bn254G1Add | Self::Bn254G1Mul => CRYPTO_BN254_G1_LEN,
            Self::Secp256k1Ecrecover => 32,
            Self::Bls12381PairingCheck
            | Self::Bn254PairingCheck
            | Self::Secp256r1Verify
            | Self::KzgVerifyProof => 1,
        }
    }

    /// Import name under `fluentbase_v1preview`.
    pub const fn import_name(self) -> &'static str {
        match self {
            Self::Bls12381G1Add => "_crypto_bls12381_g1_add",
            Self::Bls12381G1Msm => "_crypto_bls12381_g1_msm",
            Self::Bls12381G2Add => "_crypto_bls12381_g2_add",
            Self::Bls12381G2Msm => "_crypto_bls12381_g2_msm",
            Self::Bls12381PairingCheck => "_crypto_bls12381_pairing_check",
            Self::Bls12381MapFpToG1 => "_crypto_bls12381_map_fp_to_g1",
            Self::Bls12381MapFp2ToG2 => "_crypto_bls12381_map_fp2_to_g2",
            Self::Bn254G1Add => "_crypto_bn254_g1_add",
            Self::Bn254G1Mul => "_crypto_bn254_g1_mul",
            Self::Bn254PairingCheck => "_crypto_bn254_pairing_check",
            Self::Secp256k1Ecrecover => "_crypto_secp256k1_ecrecover",
            Self::Secp256r1Verify => "_crypto_secp256r1_verify",
            Self::KzgVerifyProof => "_crypto_kzg_verify_proof",
        }
    }
}

/// Return codes of the crypto syscalls. `0` is success.
///
/// The codes carry the failure kind across the boundary so a guest can rebuild the
/// `PrecompileHalt` the host produced. Every non-zero code is a failure of the operation; the
/// precompile semantics only depend on success versus failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Display, FromRepr)]
#[repr(i32)]
pub enum CryptoSyscallError {
    /// Payload length or layout is not valid for the operation.
    InvalidInput = 1,
    /// A field element is not canonical (at or above the modulus).
    NonCanonicalFieldElement = 2,
    /// A point is not on its curve.
    PointNotOnCurve = 3,
    /// A point is not in the prime-order subgroup.
    PointNotInSubgroup = 4,
    /// The signature could not be recovered or did not verify.
    VerificationFailed = 5,
    /// Any other host-side failure.
    Other = 100,
}

/// Every operation, in `SysFuncIdx` order.
pub const CRYPTO_SYSCALLS: [CryptoSyscall; 13] = [
    CryptoSyscall::Bls12381G1Add,
    CryptoSyscall::Bls12381G1Msm,
    CryptoSyscall::Bls12381G2Add,
    CryptoSyscall::Bls12381G2Msm,
    CryptoSyscall::Bls12381PairingCheck,
    CryptoSyscall::Bls12381MapFpToG1,
    CryptoSyscall::Bls12381MapFp2ToG2,
    CryptoSyscall::Bn254G1Add,
    CryptoSyscall::Bn254G1Mul,
    CryptoSyscall::Bn254PairingCheck,
    CryptoSyscall::Secp256k1Ecrecover,
    CryptoSyscall::Secp256r1Verify,
    CryptoSyscall::KzgVerifyProof,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{import_linker_v1_preview, SysFuncIdx};
    use rwasm::{ImportName, ValType};

    #[test]
    fn every_operation_is_linked_with_the_uniform_signature() {
        let linker = import_linker_v1_preview();
        for op in CRYPTO_SYSCALLS {
            let name = ImportName::new("fluentbase_v1preview", op.import_name());
            let entry = linker
                .resolve_by_import_name(&name)
                .unwrap_or_else(|| panic!("{} is not linked", op.import_name()));
            assert_eq!(entry.sys_func_idx, op as u32, "{op}");
            assert_eq!(entry.params, &[ValType::I32; 3], "{op}");
            assert_eq!(entry.result, &[ValType::I32; 1], "{op}");
            assert_eq!(
                SysFuncIdx::from_repr(op as u32).map(u32::from),
                Some(op as u32)
            );
        }
    }

    #[test]
    fn payload_rules() {
        assert!(CryptoSyscall::Bls12381G1Add.accepts_input_len(192));
        assert!(!CryptoSyscall::Bls12381G1Add.accepts_input_len(191));
        assert!(CryptoSyscall::Bls12381PairingCheck.accepts_input_len(0));
        assert!(CryptoSyscall::Bls12381PairingCheck.accepts_input_len(3 * 288));
        assert!(!CryptoSyscall::Bls12381PairingCheck.accepts_input_len(3 * 288 + 1));
        assert!(!CryptoSyscall::Bls12381PairingCheck
            .accepts_input_len(CRYPTO_SYSCALL_MAX_INPUT_LEN + 288));
        assert!(CryptoSyscall::Secp256k1Ecrecover.accepts_input_len(97));
        assert_eq!(CryptoSyscall::Secp256k1Ecrecover.output_len(), 32);
        assert_eq!(CryptoSyscall::KzgVerifyProof.output_len(), 1);
        assert_eq!(
            CryptoSyscallError::from_repr(1),
            Some(CryptoSyscallError::InvalidInput)
        );
    }
}
