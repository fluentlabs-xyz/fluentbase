pub mod charge_fuel;
pub mod debug_log;
pub mod ec_recover;
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
pub mod output_size;
pub mod poseidon;
pub mod poseidon_hash;
pub mod preimage_copy;
pub mod preimage_size;
pub mod read;
pub mod read_output;
pub mod resume;
pub mod sha256_compress;
pub mod sha256_extend;
pub mod state;
pub mod uint256_mul;
pub mod weierstrass_add;
pub mod weierstrass_decompress;
pub mod weierstrass_double;
pub mod write;

use crate::{
    impl_runtime_handler,
    instruction::{
        charge_fuel::SyscallChargeFuel,
        debug_log::SyscallDebugLog,
        ec_recover::SyscallEcrecover,
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
        output_size::SyscallOutputSize,
        poseidon::SyscallPoseidon,
        poseidon_hash::SyscallPoseidonHash,
        preimage_copy::SyscallPreimageCopy,
        preimage_size::SyscallPreimageSize,
        read::SyscallRead,
        read_output::SyscallReadOutput,
        resume::SyscallResume,
        sha256_compress::SyscallSha256Compress,
        sha256_extend::SyscallSha256Extend,
        state::SyscallState,
        uint256_mul::SyscallUint256Mul,
        weierstrass_add::SyscallWeierstrassAddAssign,
        weierstrass_decompress::SyscallWeierstrassDecompressAssign,
        weierstrass_double::SyscallWeierstrassDoubleAssign,
        write::SyscallWrite,
    },
    RuntimeContext,
};
use fluentbase_types::SysFuncIdx;
use num::BigUint;
use rwasm::{Caller, Linker, Store};
use sp1_curves::{
    edwards::ed25519::Ed25519,
    weierstrass::{
        bls12_381::{Bls12381, Bls12381BaseField},
        bn254::{Bn254, Bn254BaseField},
        secp256k1::Secp256k1,
    },
};

pub trait RuntimeHandler {
    const MODULE_NAME: &'static str;
    const FUNC_NAME: &'static str;
    const FUNC_INDEX: SysFuncIdx;

    fn register_handler(linker: &mut Linker<RuntimeContext>, store: &mut Store<RuntimeContext>);
}

impl_runtime_handler!(SyscallExit, EXIT, fn fluentbase_v1preview::_exit(exit_code: i32) -> ());
impl_runtime_handler!(SyscallState, STATE, fn fluentbase_v1preview::_state() -> u32);
impl_runtime_handler!(SyscallRead, READ_INPUT, fn fluentbase_v1preview::_read(target: u32, offset: u32, length: u32) -> ());
impl_runtime_handler!(SyscallInputSize, INPUT_SIZE, fn fluentbase_v1preview::_input_size() -> u32);
impl_runtime_handler!(SyscallWrite, WRITE_OUTPUT, fn fluentbase_v1preview::_write(offset: u32, length: u32) -> ());
impl_runtime_handler!(SyscallOutputSize, OUTPUT_SIZE, fn fluentbase_v1preview::_output_size() -> u32);
impl_runtime_handler!(SyscallReadOutput, READ_OUTPUT, fn fluentbase_v1preview::_read_output(target: u32, offset: u32, length: u32) -> ());
impl_runtime_handler!(SyscallExec, EXEC, fn fluentbase_v1preview::_exec(code_hash32_ptr: u32, input_ptr: u32, input_len: u32, fuel_ptr: u32, state: u32) -> i32);
impl_runtime_handler!(SyscallResume, RESUME, fn fluentbase_v1preview::_resume(call_id: u32, return_data_ptr: u32, return_data_len: u32, exit_code: i32, fuel_ptr: u32) -> i32);
impl_runtime_handler!(SyscallForwardOutput, FORWARD_OUTPUT, fn fluentbase_v1preview::_forward_output(offset: u32, len: u32) -> ());
impl_runtime_handler!(SyscallChargeFuel, CHARGE_FUEL, fn fluentbase_v1preview::_charge_fuel(delta: u64) -> u64);
impl_runtime_handler!(SyscallFuel, FUEL, fn fluentbase_v1preview::_fuel() -> u64);
impl_runtime_handler!(SyscallPreimageSize, PREIMAGE_SIZE, fn fluentbase_v1preview::_preimage_size(hash32_ptr: u32) -> u32);
impl_runtime_handler!(SyscallPreimageCopy, PREIMAGE_COPY, fn fluentbase_v1preview::_preimage_copy(hash32_ptr: u32, preimage_ptr: u32) -> ());
impl_runtime_handler!(SyscallDebugLog, DEBUG_LOG, fn fluentbase_v1preview::_debug_log(msg_ptr: u32, msg_len: u32) -> ());

impl_runtime_handler!(SyscallKeccak256, KECCAK256, fn fluentbase_v1preview::_keccak256(data_ptr: u32, data_len: u32, output_ptr: u32) -> ());
impl_runtime_handler!(SyscallKeccak256Permute, KECCAK256_PERMUTE, fn  fluentbase_v1preview::_keccak256_permute(state_ptr: u32) -> ());
impl_runtime_handler!(SyscallPoseidon, POSEIDON, fn fluentbase_v1preview::_poseidon(f32s_ptr: u32, f32s_len: u32, output_ptr: u32) -> ());
impl_runtime_handler!(SyscallPoseidonHash, POSEIDON_HASH, fn fluentbase_v1preview::_poseidon_hash(fa32_ptr: u32, fb32_ptr: u32, fd32_ptr: u32, output_ptr: u32) -> ());
impl_runtime_handler!(SyscallSha256Extend, SHA256_EXTEND, fn fluentbase_v1preview::_sha256_extend(w_ptr: u32) -> ());
impl_runtime_handler!(SyscallSha256Compress, SHA256_COMPRESS, fn  fluentbase_v1preview::_sha256_compress(w_ptr: u32, h_ptr: u32) -> ());

impl_runtime_handler!(SyscallEdwardsAddAssign<Ed25519>, ED25519_ADD, fn  fluentbase_v1preview::_ed25519_add(p_ptr: u32, q_ptr: u32) -> ());
impl_runtime_handler!(SyscallEdwardsDecompress<Ed25519>, ED25519_DECOMPRESS, fn  fluentbase_v1preview::_ed25519_decompress(slice_ptr: u32, sign: u32) -> ());

impl_runtime_handler!(SyscallEcrecover, SECP256K1_RECOVER, fn fluentbase_v1preview::_ecrecover(digest32_ptr: u32, sig64_ptr: u32, output65_ptr: u32, rec_id: u32) -> ());
impl_runtime_handler!(SyscallWeierstrassAddAssign<Secp256k1>, SECP256K1_ADD, fn  fluentbase_v1preview::_secp256k1_add(p_ptr: u32, q_ptr: u32) -> ());
impl_runtime_handler!(SyscallWeierstrassDecompressAssign<Secp256k1>, SECP256K1_DECOMPRESS, fn  fluentbase_v1preview::_secp256k1_decompress(x_ptr: u32, sign_bit: u32) -> ());
impl_runtime_handler!(SyscallWeierstrassDoubleAssign<Secp256k1>, SECP256K1_DOUBLE, fn  fluentbase_v1preview::_secp256k1_double(p_ptr: u32) -> ());

impl_runtime_handler!(SyscallWeierstrassDecompressAssign<Bls12381>, BLS12381_DECOMPRESS, fn  fluentbase_v1preview::_bls12381_decompress(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallWeierstrassAddAssign<Bls12381>, BLS12381_ADD, fn  fluentbase_v1preview::_bls12381_add(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallWeierstrassDoubleAssign<Bls12381>, BLS12381_DOUBLE, fn  fluentbase_v1preview::_bls12381_double(p_ptr: u32) -> ());
impl_runtime_handler!(SyscallFpOp<Bls12381BaseField, FieldAdd>, BLS12381_FP_ADD, fn  fluentbase_v1preview::_bls12381_fp_add(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallFpOp<Bls12381BaseField, FieldSub>, BLS12381_FP_SUB, fn  fluentbase_v1preview::_bls12381_fp_sub(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallFpOp<Bls12381BaseField, FieldMul>, BLS12381_FP_MUL, fn  fluentbase_v1preview::_bls12381_fp_mul(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallFp2AddSub<Bls12381BaseField, FieldAdd>, BLS12381_FP2_ADD, fn  fluentbase_v1preview::_bls12381_fp2_add(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallFp2AddSub<Bls12381BaseField, FieldSub>, BLS12381_FP2_SUB, fn  fluentbase_v1preview::_bls12381_fp2_sub(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallFp2Mul<Bls12381BaseField>, BLS12381_FP2_MUL, fn  fluentbase_v1preview::_bls12381_fp2_mul(arg1: u32, arg2: u32) -> ());

impl_runtime_handler!(SyscallWeierstrassAddAssign<Bn254>, BN254_ADD, fn  fluentbase_v1preview::_bn254_add(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallWeierstrassDoubleAssign<Bn254>, BN254_DOUBLE, fn  fluentbase_v1preview::_bn254_double(p_ptr: u32) -> ());
impl_runtime_handler!(SyscallFpOp<Bn254BaseField, FieldAdd>, BN254_FP_ADD, fn  fluentbase_v1preview::_bn254_fp_add(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallFpOp<Bn254BaseField, FieldSub>, BN254_FP_SUB, fn  fluentbase_v1preview::_bn254_fp_sub(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallFpOp<Bn254BaseField, FieldMul>, BN254_FP_MUL, fn  fluentbase_v1preview::_bn254_fp_mul(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallFp2AddSub<Bn254BaseField, FieldAdd>, BN254_FP2_ADD, fn  fluentbase_v1preview::_bn254_fp2_add(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallFp2AddSub<Bn254BaseField, FieldSub>, BN254_FP2_SUB, fn  fluentbase_v1preview::_bn254_fp2_sub(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallFp2Mul<Bn254BaseField>, BN254_FP2_MUL, fn  fluentbase_v1preview::_bn254_fp2_mul(arg1: u32, arg2: u32) -> ());

impl_runtime_handler!(SyscallUint256Mul, UINT256_MUL, fn  fluentbase_v1preview::_uint256_mul(x_ptr: u32, y_ptr: u32, m_ptr: u32) -> ());

pub fn runtime_register_handlers(
    linker: &mut Linker<RuntimeContext>,
    store: &mut Store<RuntimeContext>,
) {
    SyscallKeccak256::register_handler(linker, store);
    SyscallPoseidon::register_handler(linker, store);
    SyscallPoseidonHash::register_handler(linker, store);
    SyscallEcrecover::register_handler(linker, store);
    SyscallExit::register_handler(linker, store);
    SyscallState::register_handler(linker, store);
    SyscallRead::register_handler(linker, store);
    SyscallInputSize::register_handler(linker, store);
    SyscallWrite::register_handler(linker, store);
    SyscallOutputSize::register_handler(linker, store);
    SyscallReadOutput::register_handler(linker, store);
    SyscallExec::register_handler(linker, store);
    SyscallResume::register_handler(linker, store);
    SyscallForwardOutput::register_handler(linker, store);
    SyscallChargeFuel::register_handler(linker, store);
    SyscallFuel::register_handler(linker, store);
    SyscallPreimageSize::register_handler(linker, store);
    SyscallPreimageCopy::register_handler(linker, store);
    SyscallDebugLog::register_handler(linker, store);

    SyscallSha256Extend::register_handler(linker, store);
    SyscallSha256Compress::register_handler(linker, store);
    SyscallEdwardsAddAssign::<Ed25519>::register_handler(linker, store);
    SyscallEdwardsDecompress::<Ed25519>::register_handler(linker, store);
    SyscallKeccak256Permute::register_handler(linker, store);
    SyscallWeierstrassAddAssign::<Secp256k1>::register_handler(linker, store);
    SyscallWeierstrassDoubleAssign::<Secp256k1>::register_handler(linker, store);
    SyscallWeierstrassDecompressAssign::<Secp256k1>::register_handler(linker, store);
    SyscallWeierstrassAddAssign::<Bn254>::register_handler(linker, store);
    SyscallWeierstrassDoubleAssign::<Bn254>::register_handler(linker, store);
    SyscallWeierstrassDecompressAssign::<Bls12381>::register_handler(linker, store);
    SyscallWeierstrassAddAssign::<Bls12381>::register_handler(linker, store);
    SyscallWeierstrassDoubleAssign::<Bls12381>::register_handler(linker, store);
    SyscallUint256Mul::register_handler(linker, store);
    SyscallFpOp::<Bls12381BaseField, FieldAdd>::register_handler(linker, store);
    SyscallFpOp::<Bls12381BaseField, FieldSub>::register_handler(linker, store);
    SyscallFpOp::<Bls12381BaseField, FieldMul>::register_handler(linker, store);
    SyscallFp2AddSub::<Bls12381BaseField, FieldAdd>::register_handler(linker, store);
    SyscallFp2AddSub::<Bls12381BaseField, FieldSub>::register_handler(linker, store);
    SyscallFp2Mul::<Bls12381BaseField>::register_handler(linker, store);
    SyscallFpOp::<Bn254BaseField, FieldAdd>::register_handler(linker, store);
    SyscallFpOp::<Bn254BaseField, FieldSub>::register_handler(linker, store);
    SyscallFpOp::<Bn254BaseField, FieldMul>::register_handler(linker, store);
    SyscallFp2AddSub::<Bn254BaseField, FieldAdd>::register_handler(linker, store);
    SyscallFp2AddSub::<Bn254BaseField, FieldSub>::register_handler(linker, store);
    SyscallFp2Mul::<Bn254BaseField>::register_handler(linker, store);
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
