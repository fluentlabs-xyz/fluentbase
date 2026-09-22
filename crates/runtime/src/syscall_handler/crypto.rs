//! Handlers for the precompile-level crypto syscalls (`fluentbase_types::CryptoSyscall`).
//!
//! Each handler forwards to revm-precompile's `DefaultCrypto`, the same blst, k256 and c-kzg code
//! revm uses natively, so a guest that installs a syscall-backed provider gets byte-identical
//! results to a native node. The default provider is called directly rather than through the
//! installable `crypto()` global: in a native test binary the guest and the host share that
//! global, and a guest that installed the syscall provider would otherwise recurse into itself.
//! Engine-metered callers are charged the EIP gas of the input; self-metered system runtimes are
//! not, because the precompile guest already charges it through `sync_evm_gas`.

use crate::RuntimeContext;
use fluentbase_types::{CryptoSyscall, CryptoSyscallError, FUEL_DENOM_RATE};
use rwasm::{StoreTr, TrapCode, Value};

/// Executes one crypto syscall: `(input_ptr, input_len, output_ptr) -> code`.
pub fn syscall_crypto_handler(
    op: CryptoSyscall,
    caller: &mut impl StoreTr<RuntimeContext>,
    params: &[Value],
    result: &mut [Value],
) -> Result<(), TrapCode> {
    let input_ptr = params[0].i32().unwrap() as u32 as usize;
    let input_len = params[1].i32().unwrap() as u32 as usize;
    let output_ptr = params[2].i32().unwrap() as u32 as usize;
    if !op.accepts_input_len(input_len) {
        result[0] = Value::I32(CryptoSyscallError::InvalidInput as i32);
        return Ok(());
    }
    let input = caller.memory_read_into_vec(input_ptr, input_len)?;
    // Engine-metered callers (contracts) pay the EIP price of the operation here. Self-metered
    // system runtimes account for the same gas themselves through `sync_evm_gas`.
    if caller.data().engine_metered {
        let fuel = eip_gas(op, input_len).saturating_mul(FUEL_DENOM_RATE);
        caller.try_consume_fuel(fuel)?;
    }
    let mut output = alloc::vec![0u8; op.output_len()];
    match syscall_crypto_impl(op, &input, &mut output) {
        Ok(()) => {
            caller.memory_write(output_ptr, &output)?;
            result[0] = Value::I32(0);
        }
        Err(code) => result[0] = Value::I32(code as i32),
    }
    Ok(())
}

/// EIP gas of an operation for a payload of `input_len` bytes, as the precompiles price it.
pub fn eip_gas(op: CryptoSyscall, input_len: usize) -> u64 {
    use revm_precompile::{
        bls12_381_const::{
            DISCOUNT_TABLE_G1_MSM, DISCOUNT_TABLE_G2_MSM, G1_ADD_BASE_GAS_FEE, G1_MSM_BASE_GAS_FEE,
            G2_ADD_BASE_GAS_FEE, G2_MSM_BASE_GAS_FEE, MAP_FP2_TO_G2_BASE_GAS_FEE,
            MAP_FP_TO_G1_BASE_GAS_FEE, PAIRING_MULTIPLIER_BASE, PAIRING_OFFSET_BASE,
        },
        bls12_381_utils::msm_required_gas,
        bn254::{
            add::ISTANBUL_ADD_GAS_COST,
            mul::ISTANBUL_MUL_GAS_COST,
            pair::{ISTANBUL_PAIR_BASE, ISTANBUL_PAIR_PER_POINT},
        },
        kzg_point_evaluation::GAS_COST as KZG_GAS_COST,
        secp256r1::P256VERIFY_BASE_GAS_FEE_OSAKA,
    };
    /// EIP-2 `ecrecover` price.
    const ECRECOVER_GAS: u64 = 3_000;
    let units = |unit: usize| (input_len / unit) as u64;
    match op {
        CryptoSyscall::Bls12381G1Add => G1_ADD_BASE_GAS_FEE,
        CryptoSyscall::Bls12381G2Add => G2_ADD_BASE_GAS_FEE,
        CryptoSyscall::Bls12381MapFpToG1 => MAP_FP_TO_G1_BASE_GAS_FEE,
        CryptoSyscall::Bls12381MapFp2ToG2 => MAP_FP2_TO_G2_BASE_GAS_FEE,
        CryptoSyscall::Bls12381G1Msm => msm_required_gas(
            units(op.unit_len().unwrap()) as usize,
            &DISCOUNT_TABLE_G1_MSM,
            G1_MSM_BASE_GAS_FEE,
        ),
        CryptoSyscall::Bls12381G2Msm => msm_required_gas(
            units(op.unit_len().unwrap()) as usize,
            &DISCOUNT_TABLE_G2_MSM,
            G2_MSM_BASE_GAS_FEE,
        ),
        CryptoSyscall::Bls12381PairingCheck => {
            PAIRING_OFFSET_BASE + PAIRING_MULTIPLIER_BASE * units(op.unit_len().unwrap())
        }
        CryptoSyscall::Bn254G1Add => ISTANBUL_ADD_GAS_COST,
        CryptoSyscall::Bn254G1Mul => ISTANBUL_MUL_GAS_COST,
        CryptoSyscall::Bn254PairingCheck => {
            ISTANBUL_PAIR_BASE + ISTANBUL_PAIR_PER_POINT * units(op.unit_len().unwrap())
        }
        CryptoSyscall::Secp256k1Ecrecover => ECRECOVER_GAS,
        CryptoSyscall::Secp256r1Verify => P256VERIFY_BASE_GAS_FEE_OSAKA,
        CryptoSyscall::KzgVerifyProof => KZG_GAS_COST,
    }
}

fn halt_code(err: revm_precompile::PrecompileHalt) -> CryptoSyscallError {
    use revm_precompile::PrecompileHalt as H;
    match err {
        H::NonCanonicalFp | H::Bn254FieldPointNotAMember => {
            CryptoSyscallError::NonCanonicalFieldElement
        }
        H::Bls12381G1NotOnCurve | H::Bls12381G2NotOnCurve | H::Bn254AffineGFailedToCreate => {
            CryptoSyscallError::PointNotOnCurve
        }
        H::Bls12381G1NotInSubgroup | H::Bls12381G2NotInSubgroup => {
            CryptoSyscallError::PointNotInSubgroup
        }
        H::Secp256k1RecoverFailed | H::BlobVerifyKzgProofFailed => {
            CryptoSyscallError::VerificationFailed
        }
        _ => CryptoSyscallError::Other,
    }
}

fn fp48(bytes: &[u8]) -> [u8; 48] {
    bytes.try_into().expect("48-byte field element")
}

fn scalar32(bytes: &[u8]) -> [u8; 32] {
    bytes.try_into().expect("32-byte scalar")
}

fn g1(bytes: &[u8]) -> revm_precompile::bls12_381::G1Point {
    (fp48(&bytes[..48]), fp48(&bytes[48..96]))
}

fn g2(bytes: &[u8]) -> revm_precompile::bls12_381::G2Point {
    (
        fp48(&bytes[..48]),
        fp48(&bytes[48..96]),
        fp48(&bytes[96..144]),
        fp48(&bytes[144..192]),
    )
}

/// Runs one crypto operation on the host provider. `input` must satisfy
/// `op.accepts_input_len` and `output` must be `op.output_len()` bytes.
pub fn syscall_crypto_impl(
    op: CryptoSyscall,
    input: &[u8],
    output: &mut [u8],
) -> Result<(), CryptoSyscallError> {
    use fluentbase_types::{CRYPTO_BLS_G1_LEN, CRYPTO_BLS_G2_LEN, CRYPTO_BN254_G1_LEN};
    use revm_precompile::{Crypto, DefaultCrypto};
    debug_assert!(op.accepts_input_len(input.len()));
    debug_assert_eq!(output.len(), op.output_len());
    let crypto = DefaultCrypto;
    match op {
        CryptoSyscall::Bls12381G1Add => {
            let out = crypto
                .bls12_381_g1_add(
                    g1(&input[..CRYPTO_BLS_G1_LEN]),
                    g1(&input[CRYPTO_BLS_G1_LEN..]),
                )
                .map_err(halt_code)?;
            output.copy_from_slice(&out);
        }
        CryptoSyscall::Bls12381G1Msm => {
            let unit = op.unit_len().unwrap();
            let mut pairs = input.chunks_exact(unit).map(|c| {
                Ok((
                    g1(&c[..CRYPTO_BLS_G1_LEN]),
                    scalar32(&c[CRYPTO_BLS_G1_LEN..]),
                ))
            });
            let out = crypto.bls12_381_g1_msm(&mut pairs).map_err(halt_code)?;
            output.copy_from_slice(&out);
        }
        CryptoSyscall::Bls12381G2Add => {
            let out = crypto
                .bls12_381_g2_add(
                    g2(&input[..CRYPTO_BLS_G2_LEN]),
                    g2(&input[CRYPTO_BLS_G2_LEN..]),
                )
                .map_err(halt_code)?;
            output.copy_from_slice(&out);
        }
        CryptoSyscall::Bls12381G2Msm => {
            let unit = op.unit_len().unwrap();
            let mut pairs = input.chunks_exact(unit).map(|c| {
                Ok((
                    g2(&c[..CRYPTO_BLS_G2_LEN]),
                    scalar32(&c[CRYPTO_BLS_G2_LEN..]),
                ))
            });
            let out = crypto.bls12_381_g2_msm(&mut pairs).map_err(halt_code)?;
            output.copy_from_slice(&out);
        }
        CryptoSyscall::Bls12381PairingCheck => {
            let unit = op.unit_len().unwrap();
            let pairs: alloc::vec::Vec<_> = input
                .chunks_exact(unit)
                .map(|c| (g1(&c[..CRYPTO_BLS_G1_LEN]), g2(&c[CRYPTO_BLS_G1_LEN..])))
                .collect();
            let ok = crypto.bls12_381_pairing_check(&pairs).map_err(halt_code)?;
            output[0] = u8::from(ok);
        }
        CryptoSyscall::Bls12381MapFpToG1 => {
            let out = crypto.bls12_381_fp_to_g1(&fp48(input)).map_err(halt_code)?;
            output.copy_from_slice(&out);
        }
        CryptoSyscall::Bls12381MapFp2ToG2 => {
            let out = crypto
                .bls12_381_fp2_to_g2((fp48(&input[..48]), fp48(&input[48..])))
                .map_err(halt_code)?;
            output.copy_from_slice(&out);
        }
        CryptoSyscall::Bn254G1Add => {
            let out = crypto
                .bn254_g1_add(&input[..CRYPTO_BN254_G1_LEN], &input[CRYPTO_BN254_G1_LEN..])
                .map_err(halt_code)?;
            output.copy_from_slice(&out);
        }
        CryptoSyscall::Bn254G1Mul => {
            let out = crypto
                .bn254_g1_mul(&input[..CRYPTO_BN254_G1_LEN], &input[CRYPTO_BN254_G1_LEN..])
                .map_err(halt_code)?;
            output.copy_from_slice(&out);
        }
        CryptoSyscall::Bn254PairingCheck => {
            let unit = op.unit_len().unwrap();
            let pairs: alloc::vec::Vec<(&[u8], &[u8])> = input
                .chunks_exact(unit)
                .map(|c| (&c[..CRYPTO_BN254_G1_LEN], &c[CRYPTO_BN254_G1_LEN..]))
                .collect();
            let ok = crypto.bn254_pairing_check(&pairs).map_err(halt_code)?;
            output[0] = u8::from(ok);
        }
        CryptoSyscall::Secp256k1Ecrecover => {
            let sig: [u8; 64] = input[..64].try_into().unwrap();
            let msg: [u8; 32] = input[65..97].try_into().unwrap();
            let out = crypto
                .secp256k1_ecrecover(&sig, input[64], &msg)
                .map_err(halt_code)?;
            output.copy_from_slice(&out);
        }
        CryptoSyscall::Secp256r1Verify => {
            let msg: [u8; 32] = input[..32].try_into().unwrap();
            let sig: [u8; 64] = input[32..96].try_into().unwrap();
            let pk: [u8; 64] = input[96..160].try_into().unwrap();
            output[0] = u8::from(crypto.secp256r1_verify_signature(&msg, &sig, &pk));
        }
        CryptoSyscall::KzgVerifyProof => {
            let z: [u8; 32] = input[..32].try_into().unwrap();
            let y: [u8; 32] = input[32..64].try_into().unwrap();
            let commitment: [u8; 48] = input[64..112].try_into().unwrap();
            let proof: [u8; 48] = input[112..160].try_into().unwrap();
            crypto
                .verify_kzg_proof(&z, &y, &commitment, &proof)
                .map_err(halt_code)?;
            output[0] = 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_ec::{AffineRepr, CurveGroup};
    use ark_ff::{BigInteger, Field, PrimeField};
    use fluentbase_types::hex;
    use revm_precompile::{Crypto, DefaultCrypto};

    fn be48(f: ark_bls12_381::Fq) -> [u8; 48] {
        f.into_bigint().to_bytes_be().try_into().unwrap()
    }

    fn bls_g1(p: ark_bls12_381::G1Affine) -> Vec<u8> {
        let (x, y) = p.xy().unwrap();
        [be48(x), be48(y)].concat()
    }

    fn bls_g2(p: ark_bls12_381::G2Affine) -> Vec<u8> {
        let (x, y) = p.xy().unwrap();
        [be48(x.c0), be48(x.c1), be48(y.c0), be48(y.c1)].concat()
    }

    fn run(op: CryptoSyscall, input: &[u8]) -> Result<Vec<u8>, CryptoSyscallError> {
        let mut out = vec![0u8; op.output_len()];
        syscall_crypto_impl(op, input, &mut out).map(|()| out)
    }

    #[test]
    fn bls12381_operations_match_the_provider() {
        let g1 = ark_bls12_381::G1Affine::generator();
        let g2 = ark_bls12_381::G2Affine::generator();
        let two_g1 = (g1.into_group() + g1.into_group()).into_affine();

        let sum = run(
            CryptoSyscall::Bls12381G1Add,
            &[bls_g1(g1), bls_g1(g1)].concat(),
        )
        .unwrap();
        assert_eq!(sum, bls_g1(two_g1));

        let mut scalar = [0u8; 32];
        scalar[31] = 2;
        let msm = run(
            CryptoSyscall::Bls12381G1Msm,
            &[bls_g1(g1), scalar.to_vec()].concat(),
        )
        .unwrap();
        assert_eq!(msm, bls_g1(two_g1));

        let two_g2 = (g2.into_group() + g2.into_group()).into_affine();
        let sum = run(
            CryptoSyscall::Bls12381G2Add,
            &[bls_g2(g2), bls_g2(g2)].concat(),
        )
        .unwrap();
        assert_eq!(sum, bls_g2(two_g2));
        let msm = run(
            CryptoSyscall::Bls12381G2Msm,
            &[bls_g2(g2), scalar.to_vec()].concat(),
        )
        .unwrap();
        assert_eq!(msm, bls_g2(two_g2));

        // e(G1, G2) * e(-G1, G2) == 1, e(G1, G2) != 1.
        let neg_g1 = (-g1.into_group()).into_affine();
        let ok = run(
            CryptoSyscall::Bls12381PairingCheck,
            &[bls_g1(g1), bls_g2(g2), bls_g1(neg_g1), bls_g2(g2)].concat(),
        )
        .unwrap();
        assert_eq!(ok, [1]);
        let not_one = run(
            CryptoSyscall::Bls12381PairingCheck,
            &[bls_g1(g1), bls_g2(g2)].concat(),
        )
        .unwrap();
        assert_eq!(not_one, [0]);
        assert_eq!(run(CryptoSyscall::Bls12381PairingCheck, &[]).unwrap(), [1]);

        // Map functions agree with the provider called directly.
        let fp = be48(ark_bls12_381::Fq::from(7u64));
        assert_eq!(
            run(CryptoSyscall::Bls12381MapFpToG1, &fp).unwrap(),
            DefaultCrypto.bls12_381_fp_to_g1(&fp).unwrap()
        );
        assert_eq!(
            run(CryptoSyscall::Bls12381MapFp2ToG2, &[fp, fp].concat()).unwrap(),
            DefaultCrypto.bls12_381_fp2_to_g2((fp, fp)).unwrap()
        );

        // A point off the curve and a point outside the subgroup are reported as such.
        let mut off_curve = bls_g1(g1);
        off_curve[95] ^= 0x01;
        assert_eq!(
            run(
                CryptoSyscall::Bls12381G1Add,
                &[off_curve, bls_g1(g1)].concat()
            ),
            Err(CryptoSyscallError::PointNotOnCurve)
        );
        let mut non_canonical = bls_g1(g1);
        non_canonical[..48].copy_from_slice(&be48(-ark_bls12_381::Fq::ONE));
        non_canonical[0] |= 0x80;
        assert_eq!(
            run(
                CryptoSyscall::Bls12381G1Add,
                &[non_canonical, bls_g1(g1)].concat()
            ),
            Err(CryptoSyscallError::NonCanonicalFieldElement)
        );
    }

    #[test]
    fn bn254_operations_match_the_provider() {
        let g = ark_bn254::G1Affine::generator();
        let two_g = (g.into_group() + g.into_group()).into_affine();
        let enc = |p: ark_bn254::G1Affine| -> Vec<u8> {
            let (x, y) = p.xy().unwrap();
            [x.into_bigint().to_bytes_be(), y.into_bigint().to_bytes_be()].concat()
        };
        assert_eq!(
            run(CryptoSyscall::Bn254G1Add, &[enc(g), enc(g)].concat()).unwrap(),
            enc(two_g)
        );
        let mut scalar = [0u8; 32];
        scalar[31] = 2;
        assert_eq!(
            run(
                CryptoSyscall::Bn254G1Mul,
                &[enc(g), scalar.to_vec()].concat()
            )
            .unwrap(),
            enc(two_g)
        );
        let g2 = hex!("198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa");
        let neg_g = (-g.into_group()).into_affine();
        let ok = run(
            CryptoSyscall::Bn254PairingCheck,
            &[enc(g), g2.to_vec(), enc(neg_g), g2.to_vec()].concat(),
        )
        .unwrap();
        assert_eq!(ok, [1]);
        let not_one = run(
            CryptoSyscall::Bn254PairingCheck,
            &[enc(g), g2.to_vec()].concat(),
        )
        .unwrap();
        assert_eq!(not_one, [0]);
        let mut off_curve = enc(g);
        off_curve[63] = 3;
        assert!(run(CryptoSyscall::Bn254G1Add, &[off_curve, enc(g)].concat()).is_err());
    }

    #[test]
    fn signature_and_kzg_operations_match_the_provider() {
        // ecrecover vector from the guest tests: msg || v || r || s.
        let v = hex!("18c547e4f7b0f325ad1e56f57e26c745b09a3e503d86e00e5255ff7f715d3d1c000000000000000000000000000000000000000000000000000000000000001c73b1693892219d736caba55bdb67216e485557ea6b6af75f37096c9aa6a5a75feeb940b1d03b21e36b0e47e79769f095fe2ab855bd91e3a38756b7d75a9c4549");
        let mut payload = Vec::new();
        payload.extend_from_slice(&v[64..128]); // sig
        payload.push(v[63] - 27); // recid
        payload.extend_from_slice(&v[..32]); // msg
        let out = run(CryptoSyscall::Secp256k1Ecrecover, &payload).unwrap();
        assert_eq!(
            &out[12..],
            &hex!("a94f5374fce5edbc8e2a8697c15331677e6ebf0b")
        );
        // An all-zero signature cannot be recovered.
        payload[..64].fill(0);
        assert_eq!(
            run(CryptoSyscall::Secp256k1Ecrecover, &payload),
            Err(CryptoSyscallError::VerificationFailed)
        );

        // p256 vector from the guest tests: msg || r || s || x || y == msg || sig || pk.
        let p = hex!("4cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4da73bd4903f0ce3b639bbbf6e8e80d16931ff4bcf5993d58468e8fb19086e8cac36dbcd03009df8c59286b162af3bd7fcc0450c9aa81be5d10d312af6c66b1d604aebd3099c618202fcfe16ae7770b0c49ab5eadf74b754204a3bb6060e44eff37618b065f9832de4ca6ca971a7a1adc826d0f7c00181a5fb2ddf79ae00b4e10e");
        assert_eq!(run(CryptoSyscall::Secp256r1Verify, &p).unwrap(), [1]);
        let mut bad = p;
        bad[0] ^= 0x01;
        assert_eq!(run(CryptoSyscall::Secp256r1Verify, &bad).unwrap(), [0]);

        // KZG vector from the guest tests: z || y || commitment || proof.
        let z = hex!("73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000000");
        let y = hex!("1522a4a7f34e1ea350ae07c29c96c7e79655aa926122e95fe69fcbd932ca49e9");
        let commitment = hex!("8f59a8d2a1a625a17f3fea0fe5eb8c896db3764f3185481bc22f91b4aaffcca25f26936857bc3a7c2539ea8ec3a952b7");
        let proof = hex!("a62ad71d14c5719385c0686f1871430475bf3a00f0aa3f7b8dd99a9abc2160744faf0070725e00b60ad9a026a15b1a8c");
        let kzg = [z.to_vec(), y.to_vec(), commitment.to_vec(), proof.to_vec()].concat();
        assert_eq!(run(CryptoSyscall::KzgVerifyProof, &kzg).unwrap(), [1]);
        let mut bad = kzg.clone();
        bad[159] ^= 0x01;
        assert_eq!(
            run(CryptoSyscall::KzgVerifyProof, &bad),
            Err(CryptoSyscallError::VerificationFailed)
        );
    }

    #[test]
    fn eip_gas_follows_the_precompile_schedules() {
        assert_eq!(eip_gas(CryptoSyscall::Bls12381G1Add, 192), 375);
        assert_eq!(eip_gas(CryptoSyscall::Bls12381G2Add, 384), 600);
        assert_eq!(eip_gas(CryptoSyscall::Bls12381MapFpToG1, 48), 5_500);
        assert_eq!(eip_gas(CryptoSyscall::Bls12381MapFp2ToG2, 96), 23_800);
        assert_eq!(eip_gas(CryptoSyscall::Bls12381G1Msm, 128), 12_000);
        assert_eq!(
            eip_gas(CryptoSyscall::Bls12381G1Msm, 2 * 128),
            2 * 12_000 * 949 / 1000
        );
        assert_eq!(eip_gas(CryptoSyscall::Bls12381G2Msm, 224), 22_500);
        assert_eq!(eip_gas(CryptoSyscall::Bls12381PairingCheck, 0), 37_700);
        assert_eq!(
            eip_gas(CryptoSyscall::Bls12381PairingCheck, 2 * 288),
            37_700 + 2 * 32_600
        );
        assert_eq!(eip_gas(CryptoSyscall::Bn254G1Add, 128), 150);
        assert_eq!(eip_gas(CryptoSyscall::Bn254G1Mul, 96), 6_000);
        assert_eq!(
            eip_gas(CryptoSyscall::Bn254PairingCheck, 3 * 192),
            45_000 + 3 * 34_000
        );
        assert_eq!(eip_gas(CryptoSyscall::Secp256k1Ecrecover, 97), 3_000);
        assert_eq!(eip_gas(CryptoSyscall::Secp256r1Verify, 160), 6_900);
        assert_eq!(eip_gas(CryptoSyscall::KzgVerifyProof, 160), 50_000);
    }
}
