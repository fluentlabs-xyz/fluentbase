pub mod charge_fuel;
pub mod debug_log;
pub mod ecrecover;
pub mod exec;
pub mod exit;
pub mod forward_output;
pub mod fuel;
pub mod input_size;
pub mod keccak256;
pub mod output_size;
pub mod poseidon;
pub mod poseidon_hash;
pub mod preimage_copy;
pub mod preimage_size;
pub mod read;
pub mod read_output;
pub mod resume;
pub mod state;
pub mod write;
mod sp1;


use crate::{
    impl_runtime_handler,
    instruction::{
        charge_fuel::SyscallChargeFuel,
        debug_log::SyscallDebugLog,
        ecrecover::SyscallEcrecover,
        exec::SyscallExec,
        exit::SyscallExit,
        forward_output::SyscallForwardOutput,
        fuel::SyscallFuel,
        input_size::SyscallInputSize,
        keccak256::SyscallKeccak256,
        output_size::SyscallOutputSize,
        poseidon::SyscallPoseidon,
        poseidon_hash::SyscallPoseidonHash,
        preimage_copy::SyscallPreimageCopy,
        preimage_size::SyscallPreimageSize,
        read::SyscallRead,
        read_output::SyscallReadOutput,
        resume::SyscallResume,
        state::SyscallState,
        write::SyscallWrite,
        sp1::halt::SyscallHalt,
    },
    RuntimeContext,
};

use sp1_curves::{
    edwards::ed25519::{Ed25519},
    weierstrass::{
        bls12_381::{Bls12381, Bls12381BaseField},
        bn254::{Bn254, Bn254BaseField},
        secp256k1::Secp256k1,
    },
};
use fluentbase_types::SysFuncIdx;
use rwasm::{Caller, Linker, Store};
use crate::instruction::sp1::ed_add::SyscallEdwardsAddAssign;
use crate::instruction::sp1::ed_decompress::SyscallEdwardsDecompress;
use crate::instruction::sp1::fp_op::SyscallFpOp;
use crate::instruction::sp1::keccak_permute::SyscallKeccak256Permute;
use crate::instruction::sp1::sha256_compress::SyscallSha256Compress;
use crate::instruction::sp1::sha256_extend::SyscallSha256Extend;
use crate::instruction::sp1::uint256_mul::SyscallUint256Mul;
use crate::instruction::sp1::weierstrass_add::SyscallWeierstrassAddAssign;
use crate::instruction::sp1::weierstrass_decompress::SyscallWeierstrassDecompressAssign;
use crate::instruction::sp1::weierstrass_double::SyscallWeierstrassDoubleAssign;
use crate::instruction::sp1::write::SyscallWriteFd;
use crate::instruction::sp1::{Add, Mul, Sub};
use crate::instruction::sp1::fp2_addsub::SyscallFp2AddSub;
use crate::instruction::sp1::fp2_mul::SyscallFp2Mul;

pub trait RuntimeHandler {
    const MODULE_NAME: &'static str;
    const FUNC_NAME: &'static str;
    const FUNC_INDEX: SysFuncIdx;

    fn register_handler(linker: &mut Linker<RuntimeContext>, store: &mut Store<RuntimeContext>);
}

impl_runtime_handler!(SyscallKeccak256, KECCAK256, fn fluentbase_v1preview::_keccak256(data_ptr: u32, data_len: u32, output_ptr: u32) -> ());
impl_runtime_handler!(SyscallPoseidon, POSEIDON, fn fluentbase_v1preview::_poseidon(f32s_ptr: u32, f32s_len: u32, output_ptr: u32) -> ());
impl_runtime_handler!(SyscallPoseidonHash, POSEIDON_HASH, fn fluentbase_v1preview::_poseidon_hash(fa32_ptr: u32, fb32_ptr: u32, fd32_ptr: u32, output_ptr: u32) -> ());
impl_runtime_handler!(SyscallEcrecover, ECRECOVER, fn fluentbase_v1preview::_ecrecover(digest32_ptr: u32, sig64_ptr: u32, output65_ptr: u32, rec_id: u32) -> ());
impl_runtime_handler!(SyscallExit, EXIT, fn fluentbase_v1preview::_exit(exit_code: i32) -> ());
impl_runtime_handler!(SyscallState, STATE, fn fluentbase_v1preview::_state() -> u32);
impl_runtime_handler!(SyscallRead, READ, fn fluentbase_v1preview::_read(target: u32, offset: u32, length: u32) -> ());
impl_runtime_handler!(SyscallInputSize, INPUT_SIZE, fn fluentbase_v1preview::_input_size() -> u32);
impl_runtime_handler!(SyscallWrite, WRITE, fn fluentbase_v1preview::_write(offset: u32, length: u32) -> ());
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

impl_runtime_handler!(SyscallHalt, HALT, fn fluentbase_v1preview::_halt(exit_code: u32, none: u32) -> ());
impl_runtime_handler!(SyscallWriteFd, WRITE_FD, fn fluentbase_v1preview::write_fd(fd: u32, write_buf: u32, length: u32) -> ());
impl_runtime_handler!(SyscallSha256Extend, SHA_EXTEND, fn fluentbase_v1preview::_sha256Extend(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallSha256Compress, SHA_COMPRESS, fn  fluentbase_v1preview::_sha256Compress(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallEdwardsAddAssign<Ed25519>, ED_ADD, fn  fluentbase_v1preview::_edwardsAddAssign(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallEdwardsDecompress<Ed25519>, ED_DECOMPRESS, fn  fluentbase_v1preview::_edwardsDecompress(slice_ptr: u32, sign: u32) -> ());
impl_runtime_handler!(SyscallKeccak256Permute, KECCAK_PERMUTE, fn  fluentbase_v1preview::_keccak256Permute(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallWeierstrassAddAssign<Secp256k1>, SECP256K1_ADD, fn  fluentbase_v1preview::_secp256k1Add(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallWeierstrassDoubleAssign<Secp256k1>, SECP256K1_DOUBLE, fn  fluentbase_v1preview::_secp256k1Double(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallWeierstrassDecompressAssign<Secp256k1>, SECP256K1_DECOMPRESS, fn  fluentbase_v1preview::_secp256k1Decompress(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallWeierstrassAddAssign<Bn254>, BN254_ADD, fn  fluentbase_v1preview::_bn254Add(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallWeierstrassDoubleAssign<Bn254>, BN254_DOUBLE, fn  fluentbase_v1preview::_bn254Double(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallWeierstrassDecompressAssign<Bls12381>, BLS12381_DECOMPRESS, fn  fluentbase_v1preview::_bls12381Decompress(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallWeierstrassAddAssign<Bls12381>, BLS12381_ADD, fn  fluentbase_v1preview::_bls12381Add(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallWeierstrassDoubleAssign<Bls12381>, BLS12381_DOUBLE, fn  fluentbase_v1preview::_bls12381Double(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallUint256Mul, UINT256_MUL, fn  fluentbase_v1preview::SyscallUint256Mul(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallFpOp<Bls12381BaseField, Add>, BLS12381_FP_ADD, fn  fluentbase_v1preview::_bls12381FpAdd(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallFpOp<Bls12381BaseField, Sub>, BLS12381_FP_SUB, fn  fluentbase_v1preview::_bls12381FpSub(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallFpOp<Bls12381BaseField, Mul>, BLS12381_FP_MUL, fn  fluentbase_v1preview::_bls12381FpMul(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallFp2AddSub<Bls12381BaseField, Add>, BLS12381_FP2_ADD, fn  fluentbase_v1preview::_bls12381Fp2Add(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallFp2AddSub<Bls12381BaseField, Sub>, BLS12381_FP2_SUB, fn  fluentbase_v1preview::_bls12381Fp2Sub(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallFp2Mul<Bls12381BaseField>, BLS12381_FP2_MUL, fn  fluentbase_v1preview::_bls12381Fp2Mul(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallFpOp<Bn254BaseField, Add>, BN254_FP_ADD, fn  fluentbase_v1preview::_bn254FpAdd(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallFpOp<Bn254BaseField, Sub>, BN254_FP_SUB, fn  fluentbase_v1preview::_bn254FpSub(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallFpOp<Bn254BaseField, Mul>, BN254_FP_MUL, fn  fluentbase_v1preview::_bn254FpMul(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallFp2AddSub<Bn254BaseField, Add>, BN254_FP2_ADD, fn  fluentbase_v1preview::_bn254Fp2Add(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallFp2AddSub<Bn254BaseField, Sub>, BN254_FP2_SUB, fn  fluentbase_v1preview::_bn254Fp2Sub(arg1: u32, arg2: u32) -> ());
impl_runtime_handler!(SyscallFp2Mul<Bn254BaseField>, BN254_FP2_MUL, fn  fluentbase_v1preview::_bn254Fp2Mul(arg1: u32, arg2: u32) -> ());


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

    SyscallHalt::register_handler(linker, store);
    SyscallWriteFd::register_handler(linker, store);
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
    SyscallFpOp::<Bls12381BaseField, Add>::register_handler(linker, store);
    SyscallFpOp::<Bls12381BaseField, Sub>::register_handler(linker, store);
    SyscallFpOp::<Bls12381BaseField, Mul>::register_handler(linker, store);
    SyscallFp2AddSub::<Bls12381BaseField, Add>::register_handler(linker, store);
    SyscallFp2AddSub::<Bls12381BaseField, Sub>::register_handler(linker, store);
    SyscallFp2Mul::<Bls12381BaseField>::register_handler(linker, store);
    SyscallFpOp::<Bn254BaseField, Add>::register_handler(linker, store);
    SyscallFpOp::<Bn254BaseField, Sub>::register_handler(linker, store);
    SyscallFpOp::<Bn254BaseField, Mul>::register_handler(linker, store);
    SyscallFp2AddSub::<Bn254BaseField, Add>::register_handler(linker, store);
    SyscallFp2AddSub::<Bn254BaseField, Sub>::register_handler(linker, store);
    SyscallFp2Mul::<Bn254BaseField>::register_handler(linker, store);
}
