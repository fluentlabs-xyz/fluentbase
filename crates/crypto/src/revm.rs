//! revm-precompile `Crypto` provider over the precompile-level crypto syscalls.
//!
//! A precompile guest installs it once at entry with
//! `revm_precompile::install_crypto(PrecompileCrypto)`. Every heavy operation revm-precompile
//! performs then crosses to the host in one `_crypto_*` syscall, while the EIP gas schedule, input
//! parsing and error semantics stay in revm-precompile. On the node the host side is revm's own
//! default provider; on a native target the syscalls resolve to it directly.

use crate::CryptoRuntime;
use alloc::vec::Vec;
use fluentbase_types::{CryptoAPI, CryptoSyscall};
use revm_precompile::{
    bls12_381::{G1Point, G1PointScalar, G2Point, G2PointScalar},
    Crypto, PrecompileHalt,
};

/// `Crypto` provider whose operations are the `_crypto_*` syscalls.
#[derive(Debug, Clone, Copy, Default)]
pub struct PrecompileCrypto;

/// Runs one operation on the host. `N` is the operation's output size; any non-zero code is a
/// failure of the operation, which is all the precompile semantics depend on.
fn call<const N: usize>(op: CryptoSyscall, input: &[u8]) -> Result<[u8; N], PrecompileHalt> {
    debug_assert_eq!(N, op.output_len());
    let mut output = [0u8; N];
    match CryptoRuntime::crypto_syscall(op, input, &mut output) {
        0 => Ok(output),
        _ => Err(PrecompileHalt::Other("crypto syscall failed".into())),
    }
}

fn concat<'a>(parts: impl IntoIterator<Item = &'a [u8]>) -> Vec<u8> {
    parts.into_iter().fold(Vec::new(), |mut acc, part| {
        acc.extend_from_slice(part);
        acc
    })
}

fn g1(p: &G1Point) -> [&[u8]; 2] {
    [&p.0, &p.1]
}

fn g2(p: &G2Point) -> [&[u8]; 4] {
    [&p.0, &p.1, &p.2, &p.3]
}

impl Crypto for PrecompileCrypto {
    fn bn254_g1_add(&self, p1: &[u8], p2: &[u8]) -> Result<[u8; 64], PrecompileHalt> {
        call(CryptoSyscall::Bn254G1Add, &concat([p1, p2]))
    }

    fn bn254_g1_mul(&self, point: &[u8], scalar: &[u8]) -> Result<[u8; 64], PrecompileHalt> {
        call(CryptoSyscall::Bn254G1Mul, &concat([point, scalar]))
    }

    fn bn254_pairing_check(&self, pairs: &[(&[u8], &[u8])]) -> Result<bool, PrecompileHalt> {
        let input = concat(pairs.iter().flat_map(|(g1, g2)| [*g1, *g2]));
        call::<1>(CryptoSyscall::Bn254PairingCheck, &input).map(|out| out[0] == 1)
    }

    fn secp256k1_ecrecover(
        &self,
        sig: &[u8; 64],
        recid: u8,
        msg: &[u8; 32],
    ) -> Result<[u8; 32], PrecompileHalt> {
        call(
            CryptoSyscall::Secp256k1Ecrecover,
            &concat([&sig[..], &[recid], &msg[..]]),
        )
    }

    fn secp256r1_verify_signature(&self, msg: &[u8; 32], sig: &[u8; 64], pk: &[u8; 64]) -> bool {
        call::<1>(CryptoSyscall::Secp256r1Verify, &concat([&msg[..], sig, pk]))
            .is_ok_and(|out| out[0] == 1)
    }

    fn verify_kzg_proof(
        &self,
        z: &[u8; 32],
        y: &[u8; 32],
        commitment: &[u8; 48],
        proof: &[u8; 48],
    ) -> Result<(), PrecompileHalt> {
        call::<1>(
            CryptoSyscall::KzgVerifyProof,
            &concat([&z[..], y, commitment, proof]),
        )
        .map(|_| ())
    }

    fn bls12_381_g1_add(&self, a: G1Point, b: G1Point) -> Result<[u8; 96], PrecompileHalt> {
        call(
            CryptoSyscall::Bls12381G1Add,
            &concat(g1(&a).into_iter().chain(g1(&b))),
        )
    }

    fn bls12_381_g1_msm(
        &self,
        pairs: &mut dyn Iterator<Item = Result<G1PointScalar, PrecompileHalt>>,
    ) -> Result<[u8; 96], PrecompileHalt> {
        let mut input = Vec::new();
        for pair in pairs {
            let (point, scalar) = pair?;
            input.extend(concat(g1(&point).into_iter().chain([&scalar[..]])));
        }
        call(CryptoSyscall::Bls12381G1Msm, &input)
    }

    fn bls12_381_g2_add(&self, a: G2Point, b: G2Point) -> Result<[u8; 192], PrecompileHalt> {
        call(
            CryptoSyscall::Bls12381G2Add,
            &concat(g2(&a).into_iter().chain(g2(&b))),
        )
    }

    fn bls12_381_g2_msm(
        &self,
        pairs: &mut dyn Iterator<Item = Result<G2PointScalar, PrecompileHalt>>,
    ) -> Result<[u8; 192], PrecompileHalt> {
        let mut input = Vec::new();
        for pair in pairs {
            let (point, scalar) = pair?;
            input.extend(concat(g2(&point).into_iter().chain([&scalar[..]])));
        }
        call(CryptoSyscall::Bls12381G2Msm, &input)
    }

    fn bls12_381_pairing_check(
        &self,
        pairs: &[(G1Point, G2Point)],
    ) -> Result<bool, PrecompileHalt> {
        let input = concat(
            pairs
                .iter()
                .flat_map(|(p, q)| g1(p).into_iter().chain(g2(q))),
        );
        call::<1>(CryptoSyscall::Bls12381PairingCheck, &input).map(|out| out[0] == 1)
    }

    fn bls12_381_fp_to_g1(&self, fp: &[u8; 48]) -> Result<[u8; 96], PrecompileHalt> {
        call(CryptoSyscall::Bls12381MapFpToG1, fp)
    }

    fn bls12_381_fp2_to_g2(&self, fp2: ([u8; 48], [u8; 48])) -> Result<[u8; 192], PrecompileHalt> {
        call(
            CryptoSyscall::Bls12381MapFp2ToG2,
            &concat([&fp2.0[..], &fp2.1]),
        )
    }
}
