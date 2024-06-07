pub mod charge_fuel;
pub mod checkpoint;
pub mod commit;
pub mod compute_root;
pub mod context_call;
pub mod debug_log;
pub mod ecrecover;
pub mod emit_log;
pub mod exec;
pub mod exit;
pub mod forward_output;
pub mod get_leaf;
pub mod input_size;
pub mod keccak256;
pub mod output_size;
pub mod poseidon;
pub mod poseidon_hash;
pub mod preimage_copy;
pub mod preimage_size;
pub mod read;
pub mod read_context;
pub mod read_output;
pub mod rollback;
pub mod state;
pub mod update_leaf;
pub mod update_preimage;
pub mod write;

use crate::{
    impl_runtime_handler,
    instruction::{
        charge_fuel::SyscallChargeFuel,
        checkpoint::SyscallCheckpoint,
        commit::SyscallCommit,
        compute_root::SyscallComputeRoot,
        context_call::SyscallContextCall,
        debug_log::SyscallDebugLog,
        ecrecover::SyscallEcrecover,
        emit_log::SyscallEmitLog,
        exec::SyscallExec,
        exit::SyscallExit,
        forward_output::SyscallForwardOutput,
        get_leaf::SyscallGetLeaf,
        input_size::SyscallInputSize,
        keccak256::SyscallKeccak256,
        output_size::SyscallOutputSize,
        poseidon::SyscallPoseidon,
        poseidon_hash::SyscallPoseidonHash,
        preimage_copy::SyscallPreimageCopy,
        preimage_size::SyscallPreimageSize,
        read::SyscallRead,
        read_context::SyscallReadContext,
        read_output::SyscallReadOutput,
        rollback::SyscallRollback,
        state::SyscallState,
        update_leaf::SyscallUpdateLeaf,
        update_preimage::SyscallUpdatePreimage,
        write::SyscallWrite,
    },
    RuntimeContext,
};
use fluentbase_types::{IJournaledTrie, SysFuncIdx};
use rwasm::{Caller, Linker, Store};

pub trait RuntimeHandler {
    const MODULE_NAME: &'static str;
    const FUNC_NAME: &'static str;
    const FUNC_INDEX: SysFuncIdx;

    fn register_handler<DB: IJournaledTrie>(
        linker: &mut Linker<RuntimeContext<DB>>,
        store: &mut Store<RuntimeContext<DB>>,
    );
}

impl_runtime_handler!(SyscallKeccak256, KECCAK256, fn fluentbase_v1preview::_keccak256(data_ptr: u32, data_len: u32, output_ptr: u32) -> ());
impl_runtime_handler!(SyscallPoseidon, POSEIDON, fn fluentbase_v1preview::_poseidon(f32s_ptr: u32, f32s_len: u32, output_ptr: u32) -> ());
impl_runtime_handler!(SyscallPoseidonHash, POSEIDON_HASH, fn fluentbase_v1preview::_poseidon_hash(fa32_ptr: u32, fb32_ptr: u32, fd32_ptr: u32, output_ptr: u32) -> ());
impl_runtime_handler!(SyscallEcrecover, ECRECOVER, fn fluentbase_v1preview::_ecrecover(digest32_ptr: u32, sig64_ptr: u32, output65_ptr: u32, rec_id: u32) -> ());
impl_runtime_handler!(SyscallExit, EXIT, fn fluentbase_v1preview::_exit(exit_code: i32) -> ());
impl_runtime_handler!(SyscallWrite, WRITE, fn fluentbase_v1preview::_write(offset: u32, length: u32) -> ());
impl_runtime_handler!(SyscallInputSize, INPUT_SIZE, fn fluentbase_v1preview::_input_size() -> u32);
impl_runtime_handler!(SyscallRead, READ, fn fluentbase_v1preview::_read(target: u32, offset: u32, length: u32) -> ());
impl_runtime_handler!(SyscallOutputSize, OUTPUT_SIZE, fn fluentbase_v1preview::_output_size() -> u32);
impl_runtime_handler!(SyscallReadOutput, READ_OUTPUT, fn fluentbase_v1preview::_read_output(target: u32, offset: u32, length: u32) -> ());
impl_runtime_handler!(SyscallState, STATE, fn fluentbase_v1preview::_state() -> u32);
impl_runtime_handler!(SyscallExec, EXEC, fn fluentbase_v1preview::_exec(code_hash32_ptr: u32, input_ptr: u32, input_len: u32, return_ptr: u32, return_len: u32, fuel_ptr: u32) -> i32);
impl_runtime_handler!(SyscallForwardOutput, FORWARD_OUTPUT, fn fluentbase_v1preview::_forward_output(offset: u32, len: u32) -> ());
impl_runtime_handler!(SyscallChargeFuel, CHARGE_FUEL, fn fluentbase_v1preview::_charge_fuel(delta: u64) -> u64);
impl_runtime_handler!(SyscallReadContext, READ_CONTEXT, fn fluentbase_v1preview::_read_context(target_ptr: u32, offset: u32, length: u32) -> ());
impl_runtime_handler!(SyscallContextCall, CONTEXT_CALL, fn fluentbase_v1preview::_context_call(code_hash32_ptr: u32, input_ptr: u32, input_len: u32, context_ptr: u32, context_len: u32, return_ptr: u32, return_len: u32, fuel_ptr: u32, state: u32) -> i32);
impl_runtime_handler!(SyscallCheckpoint, CHECKPOINT, fn fluentbase_v1preview::_checkpoint() -> u64);
impl_runtime_handler!(SyscallGetLeaf, GET_LEAF, fn fluentbase_v1preview::_get_leaf(key32_ptr: u32, field: u32, output32_ptr: u32, committed: u32) -> u32);
impl_runtime_handler!(SyscallUpdateLeaf, UPDATE_LEAF, fn fluentbase_v1preview::_update_leaf(key32_ptr: u32, flags: u32, vals32_ptr: u32, vals32_len: u32) -> ());
impl_runtime_handler!(SyscallComputeRoot, COMPUTE_ROOT, fn fluentbase_v1preview::_compute_root(output32_ptr: u32) -> ());
impl_runtime_handler!(SyscallEmitLog, EMIT_LOG, fn fluentbase_v1preview::_emit_log(key32_ptr: u32, topics32s_ptr: u32, topics32s_len: u32, data_ptr: u32, data_len: u32) -> ());
impl_runtime_handler!(SyscallCommit, COMMIT, fn fluentbase_v1preview::_commit(root32_ptr: u32) -> ());
impl_runtime_handler!(SyscallRollback, ROLLBACK, fn fluentbase_v1preview::_rollback(checkpoint: u64) -> ());
impl_runtime_handler!(SyscallPreimageSize, PREIMAGE_SIZE, fn fluentbase_v1preview::_preimage_size(hash32_ptr: u32) -> u32);
impl_runtime_handler!(SyscallPreimageCopy, PREIMAGE_COPY, fn fluentbase_v1preview::_preimage_copy(hash32_ptr: u32, preimage_ptr: u32) -> ());
impl_runtime_handler!(SyscallUpdatePreimage, UPDATE_PREIMAGE, fn fluentbase_v1preview::_update_preimage(key32_ptr: u32, field: u32, preimage_ptr: u32, preimage_len: u32) -> i32);
impl_runtime_handler!(SyscallDebugLog, DEBUG_LOG, fn fluentbase_v1preview::_debug_log(msg_ptr: u32, msg_len: u32) -> ());

fn runtime_register_handlers<DB: IJournaledTrie, const IS_SOVEREIGN: bool>(
    linker: &mut Linker<RuntimeContext<DB>>,
    store: &mut Store<RuntimeContext<DB>>,
) {
    SyscallKeccak256::register_handler(linker, store);
    SyscallPoseidon::register_handler(linker, store);
    SyscallPoseidonHash::register_handler(linker, store);
    SyscallEcrecover::register_handler(linker, store);
    SyscallExit::register_handler(linker, store);
    SyscallWrite::register_handler(linker, store);
    SyscallForwardOutput::register_handler(linker, store);
    SyscallInputSize::register_handler(linker, store);
    SyscallRead::register_handler(linker, store);
    SyscallOutputSize::register_handler(linker, store);
    SyscallReadOutput::register_handler(linker, store);
    SyscallExec::register_handler(linker, store);
    SyscallState::register_handler(linker, store);
    SyscallChargeFuel::register_handler(linker, store);
    SyscallReadContext::register_handler(linker, store);
    if IS_SOVEREIGN {
        SyscallContextCall::register_handler(linker, store);
        SyscallCheckpoint::register_handler(linker, store);
        SyscallUpdateLeaf::register_handler(linker, store);
        SyscallComputeRoot::register_handler(linker, store);
    }
    SyscallGetLeaf::register_handler(linker, store);
    SyscallEmitLog::register_handler(linker, store);
    if IS_SOVEREIGN {
        SyscallCommit::register_handler(linker, store);
        SyscallRollback::register_handler(linker, store);
    }
    if IS_SOVEREIGN {
        SyscallPreimageSize::register_handler(linker, store);
        SyscallUpdatePreimage::register_handler(linker, store);
    }
    SyscallPreimageCopy::register_handler(linker, store);
    SyscallDebugLog::register_handler(linker, store);
}

pub fn runtime_register_sovereign_handlers<DB: IJournaledTrie>(
    linker: &mut Linker<RuntimeContext<DB>>,
    store: &mut Store<RuntimeContext<DB>>,
) {
    runtime_register_handlers::<DB, true>(linker, store);
}

pub fn runtime_register_shared_handlers<DB: IJournaledTrie>(
    linker: &mut Linker<RuntimeContext<DB>>,
    store: &mut Store<RuntimeContext<DB>>,
) {
    runtime_register_handlers::<DB, false>(linker, store);
}
