use crate::RuntimeContext;
use fluentbase_types::{ExitCode, SysFuncIdx};
use rwasm::{Store, TrapCode, TypedCaller, Value};
use sp1_curves::weierstrass::{
    bn254::{Bn254, Bn254BaseField},
    secp256k1::Secp256k1,
};

mod bls12381;
pub use bls12381::*;
mod bn254;
pub use bn254::*;
mod ed25519;
pub use ed25519::*;
mod ristretto255;
pub use ristretto255::*;
mod secp256r1;
pub use secp256r1::*;
mod host;
pub use host::*;
mod hashing;
pub use hashing::*;
mod secp256k1;
pub use secp256k1::*;
mod bigint;
pub use bigint::*;
mod weierstrass;
pub use weierstrass::*;

/// Routes a syscall identified by func_idx to the corresponding runtime instruction handler.
pub(crate) fn runtime_syscall_handler(
    caller: &mut TypedCaller<RuntimeContext>,
    func_idx: u32,
    params: &[Value],
    result: &mut [Value],
) -> Result<(), TrapCode> {
    let sys_func_idx = SysFuncIdx::from_repr(func_idx).ok_or(TrapCode::UnknownExternalFunction)?;
    invoke_runtime_handler(caller, sys_func_idx, params, result)
}

#[rustfmt::skip]
/// Dispatches a system function index to its corresponding syscall handler.
///
/// This is the central runtime syscall dispatcher used by runtime_syscall_handler.
/// It routes the call based on SysFuncIdx without performing additional validation.
pub fn invoke_runtime_handler(
    caller: &mut TypedCaller<RuntimeContext>,
    sys_func_idx: SysFuncIdx,
    params: &[Value],
    result: &mut [Value],
) -> Result<(), TrapCode> {
    match sys_func_idx {
        // input/output & state control (0x00)
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
        SysFuncIdx::FUEL => SyscallFuel::fn_handler(caller, params, result),
        SysFuncIdx::PREIMAGE_SIZE => SyscallPreimageSize::fn_handler(caller, params, result),
        SysFuncIdx::PREIMAGE_COPY => SyscallPreimageCopy::fn_handler(caller, params, result),
        SysFuncIdx::DEBUG_LOG => SyscallDebugLog::fn_handler(caller, params, result),
        SysFuncIdx::CHARGE_FUEL => SyscallChargeFuel::fn_handler(caller, params, result),

        // hashing functions (0x01)
        SysFuncIdx::KECCAK256 => SyscallKeccak256::fn_handler(caller, params, result),
        SysFuncIdx::KECCAK256_PERMUTE => SyscallKeccak256Permute::fn_handler(caller, params, result),
        SysFuncIdx::POSEIDON => SyscallPoseidon::fn_handler(caller, params, result),
        SysFuncIdx::SHA256_EXTEND => SyscallSha256Extend::fn_handler(caller, params, result),
        SysFuncIdx::SHA256_COMPRESS => SyscallSha256Compress::fn_handler(caller, params, result),
        SysFuncIdx::SHA256 => SyscallSha256::fn_handler(caller, params, result),
        SysFuncIdx::BLAKE3 => SyscallBlake3::fn_handler(caller, params, result),

        // ed25519 (0x02)
        SysFuncIdx::ED25519_DECOMPRESS => SyscallCurve25519EdwardsDecompressValidate::fn_handler(caller, params, result),
        SysFuncIdx::ED25519_ADD => SyscallCurve25519EdwardsAdd::fn_handler(caller, params, result),
        SysFuncIdx::ED25519_SUB => SyscallCurve25519EdwardsSub::fn_handler(caller, params, result),
        SysFuncIdx::ED25519_MULTISCALAR_MUL => SyscallCurve25519EdwardsMultiscalarMul::fn_handler(caller, params, result),
        SysFuncIdx::ED25519_MUL => SyscallCurve25519EdwardsMul::fn_handler(caller, params, result),

        // ristretto255 (0x03)
        SysFuncIdx::RISTRETTO255_DECOMPRESS => SyscallCurve25519RistrettoDecompressValidate::fn_handler(caller, params, result),
        SysFuncIdx::RISTRETTO255_ADD => SyscallCurve25519RistrettoAdd::fn_handler(caller, params, result),
        SysFuncIdx::RISTRETTO255_SUB => SyscallCurve25519RistrettoSub::fn_handler(caller, params, result),
        SysFuncIdx::RISTRETTO255_MULTISCALAR_MUL => SyscallCurve25519RistrettoMultiscalarMul::fn_handler(caller, params, result),
        SysFuncIdx::RISTRETTO255_MUL => SyscallCurve25519RistrettoMul::fn_handler(caller, params, result),

        // secp256k1 (0x04)
        SysFuncIdx::SECP256K1_RECOVER => SyscallSecp256k1Recover::fn_handler(caller, params, result),
        SysFuncIdx::SECP256K1_ADD => SyscallWeierstrassAddAssign::<Secp256k1>::fn_handler(caller, params, result),
        SysFuncIdx::SECP256K1_DECOMPRESS => SyscallWeierstrassDecompressAssign::<Secp256k1>::fn_handler(caller, params, result),
        SysFuncIdx::SECP256K1_DOUBLE => SyscallWeierstrassDoubleAssign::<Secp256k1>::fn_handler(caller, params, result),

        // secp256r1 (0x05)
        SysFuncIdx::SECP256R1_VERIFY => SyscallCurve256r1Verify::fn_handler(caller, params, result),

        // bls12381 (0x06)
        SysFuncIdx::BLS12381_G1_ADD => SyscallBls12381G1Add::fn_handler(caller, params, result),
        SysFuncIdx::BLS12381_G1_MSM => SyscallBls12381G1Msm::fn_handler(caller, params, result),
        SysFuncIdx::BLS12381_G2_ADD => SyscallBls12381G2Add::fn_handler(caller, params, result),
        SysFuncIdx::BLS12381_G2_MSM => SyscallBls12381G2Msm::fn_handler(caller, params, result),
        SysFuncIdx::BLS12381_PAIRING => SyscallBls12381Pairing::fn_handler(caller, params, result),
        SysFuncIdx::BLS12381_MAP_G1 => SyscallBls12381MapFpToG1::fn_handler(caller, params, result),
        SysFuncIdx::BLS12381_MAP_G2 => SyscallBls12381MapFp2ToG2::fn_handler(caller, params, result),

        // bn254 (0x07)
        SysFuncIdx::BN254_ADD => SyscallBn256Add::fn_handler(caller, params, result),
        SysFuncIdx::BN254_MUL => SyscallBn256Mul::fn_handler(caller, params, result),
        SysFuncIdx::BN254_MULTI_PAIRING => SyscallBn256Pairing::fn_handler(caller, params, result),
        SysFuncIdx::BN254_DOUBLE => SyscallWeierstrassDoubleAssign::<Bn254>::fn_handler(caller, params, result),
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

        // uint256 (0x08)
        SysFuncIdx::BIGINT_MOD_EXP => SyscallMathBigModExp::fn_handler(caller, params, result),
        SysFuncIdx::BIGINT_UINT256_MUL => SyscallUint256Mul::fn_handler(caller, params, result),

        // sp1 (0x51)
        _ => unreachable!("unknown system function ({})", sys_func_idx),
    }
}

/// Stores the exit code in the context and converts it into a halting TrapCode.
pub(crate) fn syscall_process_exit_code(
    caller: &mut TypedCaller<RuntimeContext>,
    exit_code: ExitCode,
) -> TrapCode {
    caller.context_mut(|ctx| ctx.execution_result.exit_code = exit_code.into());
    TrapCode::ExecutionHalted
}
