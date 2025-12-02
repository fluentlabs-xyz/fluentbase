use crate::RuntimeContext;
use fluentbase_types::{ExitCode, SysFuncIdx};
use rwasm::{Store, TrapCode, TypedCaller, Value};

mod edwards;
pub use edwards::*;
mod host;
pub use host::*;
mod hashing;
pub use hashing::*;
mod uint256;
pub use uint256::*;
mod weierstrass;
pub use weierstrass::*;
mod tower;
pub use tower::*;

/// Routes a syscall identified by func_idx to the corresponding runtime instruction handler.
pub fn runtime_syscall_handler(
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
        SysFuncIdx::EXIT => syscall_exit_handler(caller, params, result),
        SysFuncIdx::STATE => syscall_state_handler(caller, params, result),
        SysFuncIdx::READ_INPUT => syscall_read_input_handler(caller, params, result),
        SysFuncIdx::INPUT_SIZE => syscall_input_size_handler(caller, params, result),
        SysFuncIdx::WRITE_OUTPUT => syscall_write_output_handler(caller, params, result),
        SysFuncIdx::OUTPUT_SIZE => syscall_output_size_handler(caller, params, result),
        SysFuncIdx::READ_OUTPUT => syscall_read_output_handler(caller, params, result),
        SysFuncIdx::EXEC => syscall_exec_handler(caller, params, result),
        SysFuncIdx::RESUME => syscall_resume_handler(caller, params, result),
        SysFuncIdx::FORWARD_OUTPUT => syscall_forward_output_handler(caller, params, result),
        SysFuncIdx::CHARGE_FUEL_MANUALLY => syscall_charge_fuel_manually_handler(caller, params, result),
        SysFuncIdx::FUEL => syscall_fuel_handler(caller, params, result),
        SysFuncIdx::DEBUG_LOG => syscall_debug_log_handler(caller, params, result),
        SysFuncIdx::CHARGE_FUEL => syscall_charge_fuel_handler(caller, params, result),
        SysFuncIdx::ENTER_UNCONSTRAINED => syscall_enter_leave_unconstrained_handler(caller, params, result),
        SysFuncIdx::EXIT_UNCONSTRAINED => syscall_enter_leave_unconstrained_handler(caller, params, result),
        // TODO(dmitry123): This syscall is disabled since it can cause panic, we should refine it
        //  by introducing new system contracts where the same functionality is achieved.
        SysFuncIdx::WRITE_FD => Err(TrapCode::UnreachableCodeReached),

        // hashing functions (0x01)
        SysFuncIdx::KECCAK256 => syscall_hashing_keccak256_handler(caller, params, result),
        SysFuncIdx::KECCAK256_PERMUTE => syscall_hashing_keccak256_permute_handler(caller, params, result),
        SysFuncIdx::POSEIDON => syscall_hashing_poseidon_handler(caller, params, result),
        SysFuncIdx::SHA256_EXTEND => syscall_hashing_sha256_extend_handler(caller, params, result),
        SysFuncIdx::SHA256_COMPRESS => syscall_hashing_sha256_compress_handler(caller, params, result),
        SysFuncIdx::SHA256 => syscall_hashing_sha256_handler(caller, params, result),
        SysFuncIdx::BLAKE3 => syscall_hashing_blake3_handler(caller, params, result),

        // ed25519 (0x02)
        SysFuncIdx::ED25519_DECOMPRESS => syscall_ed25519_decompress_handler(caller, params, result),
        SysFuncIdx::ED25519_ADD => syscall_edwards_add_handler(caller, params, result),

        // fp1/fp2 tower field (0x03)
        SysFuncIdx::TOWER_FP1_BN254_ADD => syscall_tower_fp1_bn254_add_handler(caller, params, result),
        SysFuncIdx::TOWER_FP1_BN254_SUB => syscall_tower_fp1_bn254_sub_handler(caller, params, result),
        SysFuncIdx::TOWER_FP1_BN254_MUL => syscall_tower_fp1_bn254_mul_handler(caller, params, result),
        SysFuncIdx::TOWER_FP1_BLS12381_ADD => syscall_tower_fp1_bls12381_add_handler(caller, params, result),
        SysFuncIdx::TOWER_FP1_BLS12381_SUB => syscall_tower_fp1_bls12381_sub_handler(caller, params, result),
        SysFuncIdx::TOWER_FP1_BLS12381_MUL => syscall_tower_fp1_bls12381_mul_handler(caller, params, result),
        SysFuncIdx::TOWER_FP2_BN254_ADD => syscall_tower_fp2_bn254_add_handler(caller, params, result),
        SysFuncIdx::TOWER_FP2_BN254_SUB => syscall_tower_fp2_bn254_sub_handler(caller, params, result),
        SysFuncIdx::TOWER_FP2_BN254_MUL => syscall_tower_fp2_bn254_mul_handler(caller, params, result),
        SysFuncIdx::TOWER_FP2_BLS12381_ADD => syscall_tower_fp2_bls12381_add_handler(caller, params, result),
        SysFuncIdx::TOWER_FP2_BLS12381_SUB => syscall_tower_fp2_bls12381_sub_handler(caller, params, result),
        SysFuncIdx::TOWER_FP2_BLS12381_MUL => syscall_tower_fp2_bls12381_mul_handler(caller, params, result),

        // secp256k1 (0x04)
        SysFuncIdx::SECP256K1_ADD => syscall_secp256k1_add_handler(caller, params, result),
        SysFuncIdx::SECP256K1_DECOMPRESS => syscall_secp256k1_decompress_handler(caller, params, result),
        SysFuncIdx::SECP256K1_DOUBLE => syscall_secp256k1_double_handler(caller, params, result),

        // secp256r1 (0x05)
        SysFuncIdx::SECP256R1_ADD => syscall_secp256r1_add_handler(caller, params, result),
        SysFuncIdx::SECP256R1_DECOMPRESS => syscall_secp256r1_decompress_handler(caller, params, result),
        SysFuncIdx::SECP256R1_DOUBLE => syscall_secp256r1_double_handler(caller, params, result),

        // bls12381 (0x06)
        SysFuncIdx::BLS12381_ADD => syscall_bls12381_add_handler(caller, params, result),
        SysFuncIdx::BLS12381_DECOMPRESS => syscall_bls12381_decompress_handler(caller, params, result),
        SysFuncIdx::BLS12381_DOUBLE => syscall_bls12381_double_handler(caller, params, result),

        // bn254 (0x07)
        SysFuncIdx::BN254_ADD => syscall_bn254_add_handler(caller, params, result),
        SysFuncIdx::BN254_DOUBLE => syscall_bn254_double_handler(caller, params, result),

        // uint256 (0x08)
        SysFuncIdx::UINT256_MUL_MOD => syscall_uint256_mul_mod_handler(caller, params, result),
        SysFuncIdx::UINT256_X2048_MUL => syscall_uint256_x2048_mul_handler(caller, params, result),

        // sp1 (0x51)
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
