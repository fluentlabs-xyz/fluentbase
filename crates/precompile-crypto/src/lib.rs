//! revm-precompile `Crypto` provider backed by the precompile-level crypto syscalls.
//!
//! A precompile guest installs it once at entry. From then on every heavy operation revm-precompile
//! performs, pairing checks, MSMs, signature recovery, KZG verification, crosses to the host in one
//! syscall, while the EIP gas schedule, input parsing and error semantics stay in revm-precompile
//! exactly as before. On the node the host side is revm's own `DefaultCrypto` (blst, k256, c-kzg);
//! on a native target, such as a guest's unit tests, the syscalls resolve to that implementation
//! directly.

#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;

use alloc::vec::Vec;
use fluentbase_crypto::CryptoRuntime;
use fluentbase_types::{CryptoAPI, CryptoSyscall, CryptoSyscallError};
use revm_precompile::{
    bls12_381::{G1Point, G1PointScalar, G2Point, G2PointScalar},
    install_crypto, Crypto, PrecompileHalt,
};

/// `Crypto` provider whose operations are the `_crypto_*` syscalls.
#[derive(Debug, Clone, Copy, Default)]
pub struct SyscallCrypto;

/// Installs [`SyscallCrypto`] as the process-wide provider. Returns `false` if a provider was
/// already installed, which is the normal case on every entry after the first.
pub fn install() -> bool {
    install_crypto(SyscallCrypto)
}

fn call(op: CryptoSyscall, input: &[u8]) -> Result<Vec<u8>, PrecompileHalt> {
    let mut output = alloc::vec![0u8; op.output_len()];
    match CryptoRuntime::crypto_syscall(op, input, &mut output) {
        0 => Ok(output),
        code => Err(halt(
            op,
            CryptoSyscallError::from_repr(code).unwrap_or(CryptoSyscallError::Other),
        )),
    }
}

/// Rebuilds the halt reason the host produced from the error code and the operation.
fn halt(op: CryptoSyscall, code: CryptoSyscallError) -> PrecompileHalt {
    use CryptoSyscall as Op;
    let g2 = matches!(
        op,
        Op::Bls12381G2Add | Op::Bls12381G2Msm | Op::Bls12381MapFp2ToG2
    );
    let bn254 = matches!(op, Op::Bn254G1Add | Op::Bn254G1Mul | Op::Bn254PairingCheck);
    match code {
        CryptoSyscallError::InvalidInput => {
            PrecompileHalt::Other("crypto syscall: invalid input".into())
        }
        CryptoSyscallError::NonCanonicalFieldElement if bn254 => {
            PrecompileHalt::Bn254FieldPointNotAMember
        }
        CryptoSyscallError::NonCanonicalFieldElement => PrecompileHalt::NonCanonicalFp,
        CryptoSyscallError::PointNotOnCurve if bn254 => PrecompileHalt::Bn254AffineGFailedToCreate,
        CryptoSyscallError::PointNotOnCurve if g2 => PrecompileHalt::Bls12381G2NotOnCurve,
        CryptoSyscallError::PointNotOnCurve => PrecompileHalt::Bls12381G1NotOnCurve,
        CryptoSyscallError::PointNotInSubgroup if g2 => PrecompileHalt::Bls12381G2NotInSubgroup,
        CryptoSyscallError::PointNotInSubgroup => PrecompileHalt::Bls12381G1NotInSubgroup,
        CryptoSyscallError::VerificationFailed if op == Op::KzgVerifyProof => {
            PrecompileHalt::BlobVerifyKzgProofFailed
        }
        CryptoSyscallError::VerificationFailed => PrecompileHalt::Secp256k1RecoverFailed,
        CryptoSyscallError::Other => PrecompileHalt::Other("crypto syscall failed".into()),
    }
}

fn fixed<const N: usize>(bytes: Vec<u8>) -> [u8; N] {
    bytes
        .try_into()
        .expect("output length is fixed per operation")
}

fn push_g1(buf: &mut Vec<u8>, p: &G1Point) {
    buf.extend_from_slice(&p.0);
    buf.extend_from_slice(&p.1);
}

fn push_g2(buf: &mut Vec<u8>, p: &G2Point) {
    buf.extend_from_slice(&p.0);
    buf.extend_from_slice(&p.1);
    buf.extend_from_slice(&p.2);
    buf.extend_from_slice(&p.3);
}

impl Crypto for SyscallCrypto {
    fn bn254_g1_add(&self, p1: &[u8], p2: &[u8]) -> Result<[u8; 64], PrecompileHalt> {
        let mut input = Vec::with_capacity(128);
        input.extend_from_slice(p1);
        input.extend_from_slice(p2);
        call(CryptoSyscall::Bn254G1Add, &input).map(fixed)
    }

    fn bn254_g1_mul(&self, point: &[u8], scalar: &[u8]) -> Result<[u8; 64], PrecompileHalt> {
        let mut input = Vec::with_capacity(96);
        input.extend_from_slice(point);
        input.extend_from_slice(scalar);
        call(CryptoSyscall::Bn254G1Mul, &input).map(fixed)
    }

    fn bn254_pairing_check(&self, pairs: &[(&[u8], &[u8])]) -> Result<bool, PrecompileHalt> {
        let mut input = Vec::with_capacity(pairs.len() * 192);
        for (g1, g2) in pairs {
            input.extend_from_slice(g1);
            input.extend_from_slice(g2);
        }
        call(CryptoSyscall::Bn254PairingCheck, &input).map(|out| out[0] == 1)
    }

    fn secp256k1_ecrecover(
        &self,
        sig: &[u8; 64],
        recid: u8,
        msg: &[u8; 32],
    ) -> Result<[u8; 32], PrecompileHalt> {
        let mut input = Vec::with_capacity(97);
        input.extend_from_slice(sig);
        input.push(recid);
        input.extend_from_slice(msg);
        call(CryptoSyscall::Secp256k1Ecrecover, &input).map(fixed)
    }

    fn secp256r1_verify_signature(&self, msg: &[u8; 32], sig: &[u8; 64], pk: &[u8; 64]) -> bool {
        let mut input = Vec::with_capacity(160);
        input.extend_from_slice(msg);
        input.extend_from_slice(sig);
        input.extend_from_slice(pk);
        call(CryptoSyscall::Secp256r1Verify, &input).is_ok_and(|out| out[0] == 1)
    }

    fn verify_kzg_proof(
        &self,
        z: &[u8; 32],
        y: &[u8; 32],
        commitment: &[u8; 48],
        proof: &[u8; 48],
    ) -> Result<(), PrecompileHalt> {
        let mut input = Vec::with_capacity(160);
        input.extend_from_slice(z);
        input.extend_from_slice(y);
        input.extend_from_slice(commitment);
        input.extend_from_slice(proof);
        call(CryptoSyscall::KzgVerifyProof, &input).map(|_| ())
    }

    fn bls12_381_g1_add(&self, a: G1Point, b: G1Point) -> Result<[u8; 96], PrecompileHalt> {
        let mut input = Vec::with_capacity(192);
        push_g1(&mut input, &a);
        push_g1(&mut input, &b);
        call(CryptoSyscall::Bls12381G1Add, &input).map(fixed)
    }

    fn bls12_381_g1_msm(
        &self,
        pairs: &mut dyn Iterator<Item = Result<G1PointScalar, PrecompileHalt>>,
    ) -> Result<[u8; 96], PrecompileHalt> {
        let mut input = Vec::new();
        for pair in pairs {
            let (point, scalar) = pair?;
            push_g1(&mut input, &point);
            input.extend_from_slice(&scalar);
        }
        call(CryptoSyscall::Bls12381G1Msm, &input).map(fixed)
    }

    fn bls12_381_g2_add(&self, a: G2Point, b: G2Point) -> Result<[u8; 192], PrecompileHalt> {
        let mut input = Vec::with_capacity(384);
        push_g2(&mut input, &a);
        push_g2(&mut input, &b);
        call(CryptoSyscall::Bls12381G2Add, &input).map(fixed)
    }

    fn bls12_381_g2_msm(
        &self,
        pairs: &mut dyn Iterator<Item = Result<G2PointScalar, PrecompileHalt>>,
    ) -> Result<[u8; 192], PrecompileHalt> {
        let mut input = Vec::new();
        for pair in pairs {
            let (point, scalar) = pair?;
            push_g2(&mut input, &point);
            input.extend_from_slice(&scalar);
        }
        call(CryptoSyscall::Bls12381G2Msm, &input).map(fixed)
    }

    fn bls12_381_pairing_check(
        &self,
        pairs: &[(G1Point, G2Point)],
    ) -> Result<bool, PrecompileHalt> {
        let mut input = Vec::with_capacity(pairs.len() * 288);
        for (g1, g2) in pairs {
            push_g1(&mut input, g1);
            push_g2(&mut input, g2);
        }
        call(CryptoSyscall::Bls12381PairingCheck, &input).map(|out| out[0] == 1)
    }

    fn bls12_381_fp_to_g1(&self, fp: &[u8; 48]) -> Result<[u8; 96], PrecompileHalt> {
        call(CryptoSyscall::Bls12381MapFpToG1, fp).map(fixed)
    }

    fn bls12_381_fp2_to_g2(&self, fp2: ([u8; 48], [u8; 48])) -> Result<[u8; 192], PrecompileHalt> {
        let mut input = Vec::with_capacity(96);
        input.extend_from_slice(&fp2.0);
        input.extend_from_slice(&fp2.1);
        call(CryptoSyscall::Bls12381MapFp2ToG2, &input).map(fixed)
    }
}
