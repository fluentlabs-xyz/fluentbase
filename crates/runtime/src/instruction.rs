pub mod blake3;
pub mod charge_fuel;
pub mod charge_fuel_manually;
pub mod curve25519_edwards_add;
pub mod curve25519_edwards_decompress_validate;
pub mod curve25519_edwards_mul;
pub mod curve25519_edwards_multiscalar_mul;
pub mod curve25519_edwards_sub;
pub mod curve25519_ristretto_add;
pub mod curve25519_ristretto_decompress_validate;
pub mod curve25519_ristretto_mul;
pub mod curve25519_ristretto_multiscalar_mul;
pub mod curve25519_ristretto_sub;
pub mod debug_log;
pub mod ed_add;
pub mod ed_decompress;
pub mod exec;
pub mod exit;
pub mod forward_output;
pub mod fp2_addsub;
pub mod fp2_mul;
pub mod fp_op;
pub mod fuel;
pub mod input_size;
pub mod keccak256;
pub mod keccak256_permute;
pub mod math_big_mod_exp;
pub mod output_size;
pub mod poseidon;
pub mod preimage_copy;
pub mod preimage_size;
pub mod read;
pub mod read_output;
pub mod resume;
pub mod secp256k1_recover;
pub mod sha256;
pub mod sha256_compress;
pub mod sha256_extend;
pub mod state;
pub mod uint256_mul;
pub mod weierstrass_add;
pub mod weierstrass_compress_decompress;
pub mod weierstrass_decompress;
pub mod weierstrass_double;
pub mod weierstrass_helpers;
pub mod weierstrass_mul;
pub mod weierstrass_multi_pairing;
pub mod write;

use crate::{
    instruction::{
        blake3::SyscallBlake3,
        charge_fuel::SyscallChargeFuel,
        charge_fuel_manually::SyscallChargeFuelManually,
        curve25519_edwards_add::SyscallCurve25519EdwardsAdd,
        curve25519_edwards_decompress_validate::SyscallCurve25519EdwardsDecompressValidate,
        curve25519_edwards_mul::SyscallCurve25519EdwardsMul,
        curve25519_edwards_multiscalar_mul::SyscallCurve25519EdwardsMultiscalarMul,
        curve25519_edwards_sub::SyscallCurve25519EdwardsSub,
        curve25519_ristretto_add::SyscallCurve25519RistrettoAdd,
        curve25519_ristretto_decompress_validate::SyscallCurve25519RistrettoDecompressValidate,
        curve25519_ristretto_mul::SyscallCurve25519RistrettoMul,
        curve25519_ristretto_multiscalar_mul::SyscallCurve25519RistrettoMultiscalarMul,
        curve25519_ristretto_sub::SyscallCurve25519RistrettoSub,
        debug_log::SyscallDebugLog,
        ed_add::SyscallEdwardsAddAssign,
        ed_decompress::SyscallEdwardsDecompress,
        exec::SyscallExec,
        exit::SyscallExit,
        forward_output::SyscallForwardOutput,
        fp2_addsub::SyscallFp2AddSub,
        fp2_mul::SyscallFp2Mul,
        fp_op::SyscallFpOp,
        fuel::SyscallFuel,
        input_size::SyscallInputSize,
        keccak256::SyscallKeccak256,
        keccak256_permute::SyscallKeccak256Permute,
        math_big_mod_exp::SyscallMathBigModExp,
        output_size::SyscallOutputSize,
        poseidon::SyscallPoseidon,
        preimage_copy::SyscallPreimageCopy,
        preimage_size::SyscallPreimageSize,
        read::SyscallRead,
        read_output::SyscallReadOutput,
        resume::SyscallResume,
        secp256k1_recover::SyscallSecp256k1Recover,
        sha256::SyscallSha256,
        sha256_compress::SyscallSha256Compress,
        sha256_extend::SyscallSha256Extend,
        state::SyscallState,
        uint256_mul::SyscallUint256Mul,
        weierstrass_add::SyscallWeierstrassAddAssign,
        weierstrass_compress_decompress::{
            ConfigG1Compress, ConfigG1Decompress, ConfigG2Compress, ConfigG2Decompress,
            SyscallWeierstrassCompressDecompressAssign,
        },
        weierstrass_decompress::SyscallWeierstrassDecompressAssign,
        weierstrass_double::SyscallWeierstrassDoubleAssign,
        weierstrass_mul::SyscallWeierstrassMulAssign,
        weierstrass_multi_pairing::SyscallWeierstrassMultiPairingAssign,
        write::SyscallWrite,
    },
    RuntimeContext,
};
use fluentbase_types::SysFuncIdx;
use num::BigUint;
use rwasm::{TrapCode, TypedCaller, Value};
use sp1_curves::{
    edwards::ed25519::Ed25519,
    weierstrass::{
        bls12_381::{Bls12381, Bls12381BaseField},
        bn254::{Bn254, Bn254BaseField, Bn254Parameters},
        secp256k1::Secp256k1,
    },
};

#[rustfmt::skip]
pub fn invoke_runtime_handler(
    caller: &mut TypedCaller<RuntimeContext>,
    sys_func_idx: SysFuncIdx,
    params: &[Value],
    result: &mut [Value],
) -> Result<(), TrapCode> {
    match sys_func_idx {
        SysFuncIdx::EXIT => SyscallExit::fn_handler(caller, params, result),
        SysFuncIdx::STATE => SyscallState::fn_handler(caller, params, result),
        SysFuncIdx::READ_INPUT => SyscallRead::fn_handler(caller, params, result),
        SysFuncIdx::INPUT_SIZE => SyscallInputSize::fn_handler(caller, params, result),
        SysFuncIdx::WRITE_OUTPUT => SyscallWrite::fn_handler(caller, params, result),
        SysFuncIdx::OUTPUT_SIZE => SyscallOutputSize::fn_handler(caller, params, result),
        SysFuncIdx::READ_OUTPUT => SyscallReadOutput::fn_handler(caller, params, result),
        SysFuncIdx::EXEC => SyscallExec::fn_handler(caller, params, result),
        SysFuncIdx::RESUME => SyscallResume::fn_handler(caller, params, result),
        SysFuncIdx::FORWARD_OUTPUT => SyscallForwardOutput::fn_handler(caller, params, result),
        SysFuncIdx::CHARGE_FUEL_MANUALLY => SyscallChargeFuelManually::fn_handler(caller, params, result),
        SysFuncIdx::CHARGE_FUEL => SyscallChargeFuel::fn_handler(caller, params, result),
        SysFuncIdx::FUEL => SyscallFuel::fn_handler(caller, params, result),
        SysFuncIdx::PREIMAGE_SIZE => SyscallPreimageSize::fn_handler(caller, params, result),
        SysFuncIdx::PREIMAGE_COPY => SyscallPreimageCopy::fn_handler(caller, params, result),
        SysFuncIdx::DEBUG_LOG => SyscallDebugLog::fn_handler(caller, params, result),
        SysFuncIdx::KECCAK256 => SyscallKeccak256::fn_handler(caller, params, result),
        SysFuncIdx::KECCAK256_PERMUTE => SyscallKeccak256Permute::fn_handler(caller, params, result),
        SysFuncIdx::SHA256_EXTEND => SyscallSha256Extend::fn_handler(caller, params, result),
        SysFuncIdx::SHA256_COMPRESS => SyscallSha256Compress::fn_handler(caller, params, result),
        SysFuncIdx::SHA256 => SyscallSha256::fn_handler(caller, params, result),
        SysFuncIdx::BLAKE3 => SyscallBlake3::fn_handler(caller, params, result),
        SysFuncIdx::POSEIDON => SyscallPoseidon::fn_handler(caller, params, result),
        SysFuncIdx::ED25519_ADD => SyscallEdwardsAddAssign::<Ed25519>::fn_handler(caller, params, result),
        SysFuncIdx::ED25519_DECOMPRESS => SyscallEdwardsDecompress::<Ed25519>::fn_handler(caller, params, result),
        SysFuncIdx::ED25519_EDWARDS_DECOMPRESS_VALIDATE => SyscallCurve25519EdwardsDecompressValidate::fn_handler(caller, params, result),
        SysFuncIdx::ED25519_EDWARDS_ADD => SyscallCurve25519EdwardsAdd::fn_handler(caller, params, result),
        SysFuncIdx::ED25519_EDWARDS_SUB => SyscallCurve25519EdwardsSub::fn_handler(caller, params, result),
        SysFuncIdx::ED25519_EDWARDS_MUL => SyscallCurve25519EdwardsMul::fn_handler(caller, params, result),
        SysFuncIdx::ED25519_EDWARDS_MULTISCALAR_MUL => SyscallCurve25519EdwardsMultiscalarMul::fn_handler(caller, params, result),
        SysFuncIdx::ED25519_RISTRETTO_DECOMPRESS_VALIDATE => SyscallCurve25519RistrettoDecompressValidate::fn_handler(caller, params, result),
        SysFuncIdx::ED25519_RISTRETTO_ADD => SyscallCurve25519RistrettoAdd::fn_handler(caller, params, result),
        SysFuncIdx::ED25519_RISTRETTO_SUB => SyscallCurve25519RistrettoSub::fn_handler(caller, params, result),
        SysFuncIdx::ED25519_RISTRETTO_MUL => SyscallCurve25519RistrettoMul::fn_handler(caller, params, result),
        SysFuncIdx::ED25519_RISTRETTO_MULTISCALAR_MUL => SyscallCurve25519RistrettoMultiscalarMul::fn_handler(caller, params, result),
        SysFuncIdx::SECP256K1_RECOVER => SyscallSecp256k1Recover::fn_handler(caller, params, result),
        SysFuncIdx::SECP256K1_ADD => SyscallWeierstrassAddAssign::<Secp256k1>::fn_handler(caller, params, result),
        SysFuncIdx::SECP256K1_DECOMPRESS => SyscallWeierstrassDecompressAssign::<Secp256k1>::fn_handler(caller, params, result),
        SysFuncIdx::SECP256K1_DOUBLE => SyscallWeierstrassDoubleAssign::<Secp256k1>::fn_handler(caller, params, result),
        SysFuncIdx::BLS12381_DECOMPRESS => SyscallWeierstrassDecompressAssign::<Bls12381>::fn_handler(caller, params, result),
        SysFuncIdx::BLS12381_ADD => SyscallWeierstrassAddAssign::<Bls12381>::fn_handler(caller, params, result),
        SysFuncIdx::BLS12381_DOUBLE => SyscallWeierstrassDoubleAssign::<Bls12381>::fn_handler(caller, params, result),
        SysFuncIdx::BLS12381_FP_ADD => SyscallFpOp::<Bls12381BaseField, FieldAdd>::fn_handler(caller, params, result),
        SysFuncIdx::BLS12381_FP_SUB => SyscallFpOp::<Bls12381BaseField, FieldSub>::fn_handler(caller, params, result),
        SysFuncIdx::BLS12381_FP_MUL => SyscallFpOp::<Bls12381BaseField, FieldMul>::fn_handler(caller, params, result),
        SysFuncIdx::BLS12381_FP2_ADD => SyscallFp2AddSub::<Bls12381BaseField, FieldAdd>::fn_handler(caller, params, result),
        SysFuncIdx::BLS12381_FP2_SUB => SyscallFp2AddSub::<Bls12381BaseField, FieldSub>::fn_handler(caller, params, result),
        SysFuncIdx::BLS12381_FP2_MUL => SyscallFp2Mul::<Bls12381BaseField>::fn_handler(caller, params, result),
        SysFuncIdx::BN254_ADD => SyscallWeierstrassAddAssign::<Bn254>::fn_handler(caller, params, result),
        SysFuncIdx::BN254_DOUBLE => SyscallWeierstrassDoubleAssign::<Bn254>::fn_handler(caller, params, result),
        SysFuncIdx::BN254_MUL => SyscallWeierstrassMulAssign::<Bn254Parameters>::fn_handler(caller, params, result),
        SysFuncIdx::BN254_MULTI_PAIRING => SyscallWeierstrassMultiPairingAssign::<Bn254Parameters>::fn_handler(caller, params, result),
        SysFuncIdx::BN254_G1_COMPRESS => SyscallWeierstrassCompressDecompressAssign::<ConfigG1Compress>::fn_handler(caller, params, result),
        SysFuncIdx::BN254_G1_DECOMPRESS => SyscallWeierstrassCompressDecompressAssign::<ConfigG1Decompress>::fn_handler(caller, params, result),
        SysFuncIdx::BN254_G2_COMPRESS => SyscallWeierstrassCompressDecompressAssign::<ConfigG2Compress>::fn_handler(caller, params, result),
        SysFuncIdx::BN254_G2_DECOMPRESS => SyscallWeierstrassCompressDecompressAssign::<ConfigG2Decompress>::fn_handler(caller, params, result),
        SysFuncIdx::BN254_FP_ADD => SyscallFpOp::<Bn254BaseField, FieldAdd>::fn_handler(caller, params, result),
        SysFuncIdx::BN254_FP_SUB => SyscallFpOp::<Bn254BaseField, FieldSub>::fn_handler(caller, params, result),
        SysFuncIdx::BN254_FP_MUL => SyscallFpOp::<Bn254BaseField, FieldMul>::fn_handler(caller, params, result),
        SysFuncIdx::BN254_FP2_ADD => SyscallFp2AddSub::<Bn254BaseField, FieldAdd>::fn_handler(caller, params, result),
        SysFuncIdx::BN254_FP2_SUB => SyscallFp2AddSub::<Bn254BaseField, FieldSub>::fn_handler(caller, params, result),
        SysFuncIdx::BN254_FP2_MUL => SyscallFp2Mul::<Bn254BaseField>::fn_handler(caller, params, result),
        SysFuncIdx::BIG_MOD_EXP => SyscallMathBigModExp::fn_handler(caller, params, result),
        SysFuncIdx::UINT256_MUL => SyscallUint256Mul::fn_handler(caller, params, result),
        _ => unreachable!("unknown system function ({})", sys_func_idx),
    }
}

pub fn cast_u8_to_u32(slice: &[u8]) -> Option<&[u32]> {
    if slice.len() % 4 != 0 {
        return None;
    }
    Some(unsafe { std::slice::from_raw_parts(slice.as_ptr() as *const u32, slice.len() / 4) })
}

pub trait FieldOp {
    fn execute(a: BigUint, b: BigUint, modulus: &BigUint) -> BigUint;
}

pub trait FieldOp2 {
    fn execute(
        ac0: &BigUint,
        ac1: &BigUint,
        bc0: &BigUint,
        bc1: &BigUint,
        modulus: &BigUint,
    ) -> (BigUint, BigUint);
}

pub struct FieldAdd;
impl FieldOp for FieldAdd {
    fn execute(a: BigUint, b: BigUint, modulus: &BigUint) -> BigUint {
        (a + b) % modulus
    }
}

impl FieldOp2 for FieldAdd {
    fn execute(
        ac0: &BigUint,
        ac1: &BigUint,
        bc0: &BigUint,
        bc1: &BigUint,
        modulus: &BigUint,
    ) -> (BigUint, BigUint) {
        ((ac0 + bc0) % modulus, (ac1 + bc1) % modulus)
    }
}

pub struct FieldMul;
impl FieldOp for FieldMul {
    fn execute(a: BigUint, b: BigUint, modulus: &BigUint) -> BigUint {
        (a * b) % modulus
    }
}

pub struct FieldSub;
impl FieldOp for FieldSub {
    fn execute(a: BigUint, b: BigUint, modulus: &BigUint) -> BigUint {
        ((a + modulus) - b) % modulus
    }
}

impl FieldOp2 for FieldSub {
    fn execute(
        ac0: &BigUint,
        ac1: &BigUint,
        bc0: &BigUint,
        bc1: &BigUint,
        modulus: &BigUint,
    ) -> (BigUint, BigUint) {
        (
            (ac0 + modulus - bc0) % modulus,
            (ac1 + modulus - bc1) % modulus,
        )
    }
}
