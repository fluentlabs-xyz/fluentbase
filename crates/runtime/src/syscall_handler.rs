use crate::RuntimeContext;
use fluentbase_types::{ExitCode, SysFuncIdx};
use rwasm::{Store, TrapCode, TypedCaller, Value};
use sp1_curves::weierstrass::{bls12_381::Bls12381, bn254::Bn254, secp256k1::Secp256k1};

mod edwards;
pub use edwards::*;
mod host;
pub use host::*;
mod hashing;
pub use hashing::*;
mod uint256;
pub use uint256::*;
pub mod ecc;
pub mod tower;

use crate::syscall_handler::tower::{tower_fp1_add_sub_mul, tower_fp2_add_sub_mul};
pub use ecc::*;

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
    caller: &mut impl Store<RuntimeContext>,
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
        SysFuncIdx::ED25519_DECOMPRESS => syscall_ed25519_decompress_handler(caller, params, result),
        SysFuncIdx::ED25519_ADD => syscall_edwards_add_handler(caller, params, result),
        // SysFuncIdx::ED25519_SUB => SyscallCurve25519EdwardsSub::fn_handler(caller, params, result),
        // SysFuncIdx::ED25519_MULTISCALAR_MUL => SyscallCurve25519EdwardsMultiscalarMul::fn_handler(caller, params, result),
        // SysFuncIdx::ED25519_MUL => SyscallCurve25519EdwardsMul::fn_handler(caller, params, result),

        // fp1/fp2 tower field (0x03)
        SysFuncIdx::TOWER_FP1_BN254_ADD => tower_fp1_add_sub_mul::syscall_tower_fp1_bn254_add_handler(caller, params, result),
        SysFuncIdx::TOWER_FP1_BN254_SUB => tower_fp1_add_sub_mul::syscall_tower_fp1_bn254_sub_handler(caller, params, result),
        SysFuncIdx::TOWER_FP1_BN254_MUL => tower_fp1_add_sub_mul::syscall_tower_fp1_bn254_mul_handler(caller, params, result),
        SysFuncIdx::TOWER_FP1_BLS12381_ADD => tower_fp1_add_sub_mul::syscall_tower_fp1_bls12381_add_handler(caller, params, result),
        SysFuncIdx::TOWER_FP1_BLS12381_SUB => tower_fp1_add_sub_mul::syscall_tower_fp1_bls12381_sub_handler(caller, params, result),
        SysFuncIdx::TOWER_FP1_BLS12381_MUL => tower_fp1_add_sub_mul::syscall_tower_fp1_bls12381_mul_handler(caller, params, result),
        SysFuncIdx::TOWER_FP2_BN254_ADD => tower_fp2_add_sub_mul::syscall_tower_fp2_bn254_add_handler(caller, params, result),
        SysFuncIdx::TOWER_FP2_BN254_SUB => tower_fp2_add_sub_mul::syscall_tower_fp2_bn254_sub_handler(caller, params, result),
        SysFuncIdx::TOWER_FP2_BN254_MUL => tower_fp2_add_sub_mul::syscall_tower_fp2_bn254_mul_handler(caller, params, result),
        SysFuncIdx::TOWER_FP2_BLS12381_ADD => tower_fp2_add_sub_mul::syscall_tower_fp2_bls12381_add_handler(caller, params, result),
        SysFuncIdx::TOWER_FP2_BLS12381_SUB => tower_fp2_add_sub_mul::syscall_tower_fp2_bls12381_sub_handler(caller, params, result),
        SysFuncIdx::TOWER_FP2_BLS12381_MUL => tower_fp2_add_sub_mul::syscall_tower_fp2_bls12381_mul_handler(caller, params, result),

        // secp256k1 (0x04)
        SysFuncIdx::SECP256K1_ADD => ecc_add::ecc_add_handler::<Secp256k1AddConfig>(caller, params, result),
        SysFuncIdx::SECP256K1_DECOMPRESS => SyscallEccCompressDecompress::<Secp256k1DecompressConfig>::fn_handler(caller, params, result),
        SysFuncIdx::SECP256K1_DOUBLE => SyscallEccDouble::<Secp256k1>::fn_handler(caller, params, result),

        // secp256r1 (0x05)
        // SysFuncIdx::SECP256R1_VERIFY => SyscallWeierstrassVerifyAssign::<Secp256r1VerifyConfig>::fn_handler(caller, params, result),

        // bls12381 (0x06)
        SysFuncIdx::BLS12381_G1_ADD => ecc_add::ecc_add_handler::<Bls12381G1AddConfig>(caller, params, result),
        SysFuncIdx::BLS12381_G1_MSM => SyscallEccMsm::<Bls12381G1MulConfig>::fn_handler(caller, params, result),
        SysFuncIdx::BLS12381_G2_ADD => ecc_add::ecc_add_handler::<Bls12381G2AddConfig>(caller, params, result),
        SysFuncIdx::BLS12381_G2_MSM => SyscallEccMsm::<Bls12381G2MulConfig>::fn_handler(caller, params, result),
        SysFuncIdx::BLS12381_PAIRING => SyscallEccPairing::<Bls12381>::fn_handler(caller, params, result),
        SysFuncIdx::BLS12381_MAP_G1 => SyscallEccMapping::<Bls12381G1MapConfig>:: fn_handler(caller, params, result),
        SysFuncIdx::BLS12381_MAP_G2 => SyscallEccMapping::<Bls12381G2MapConfig>::fn_handler(caller, params, result),

        // bn254 (0x07)
        SysFuncIdx::BN254_ADD => ecc_add::ecc_add_handler::<Bn254G1AddConfig>(caller, params, result),
        SysFuncIdx::BN254_MUL => SyscallEccMul::<Bn254G1MulConfig>::fn_handler(caller, params, result),
        SysFuncIdx::BN254_MULTI_PAIRING => SyscallEccPairing::<Bn254>::fn_handler(caller, params, result),
        SysFuncIdx::BN254_DOUBLE => SyscallEccDouble::<Bn254>::fn_handler(caller, params, result),
        SysFuncIdx::BN254_G1_COMPRESS => SyscallEccCompressDecompress::<Bn254G1CompressConfig>::fn_handler(caller, params, result),
        SysFuncIdx::BN254_G1_DECOMPRESS => SyscallEccCompressDecompress::<Bn254G1DecompressConfig>::fn_handler(caller, params, result),
        SysFuncIdx::BN254_G2_COMPRESS => SyscallEccCompressDecompress::<Bn254G2CompressConfig>::fn_handler(caller, params, result),
        SysFuncIdx::BN254_G2_DECOMPRESS => SyscallEccCompressDecompress::<Bn254G2DecompressConfig>::fn_handler(caller, params, result),

        // uint256 (0x08)
        SysFuncIdx::UINT256_MUL_MOD => syscall_uint256_mul_mod_handler(caller, params, result),

        // sp1 (0x51)
        _ => unreachable!("unknown system function ({})", sys_func_idx),
    }
}

/// Stores the exit code in the context and converts it into a halting TrapCode.
pub(crate) fn syscall_process_exit_code(
    ctx: &mut impl Store<RuntimeContext>,
    exit_code: ExitCode,
) -> TrapCode {
    ctx.context_mut(|ctx| ctx.execution_result.exit_code = exit_code.into());
    TrapCode::ExecutionHalted
}
