pub mod crypto_ecrecover;
pub mod crypto_keccak256;
pub mod crypto_poseidon;
pub mod crypto_poseidon2;
pub mod jzkt_checkpoint;
pub mod jzkt_commit;
pub mod jzkt_compute_root;
pub mod jzkt_emit_log;
pub mod jzkt_get;
pub mod jzkt_load;
pub mod jzkt_open;
pub mod jzkt_preimage_copy;
pub mod jzkt_preimage_size;
pub mod jzkt_remove;
pub mod jzkt_rollback;
pub mod jzkt_store;
pub mod jzkt_update;
pub mod jzkt_update_preimage;
pub mod rwasm_compile;
pub mod rwasm_create;
pub mod rwasm_transact;
pub mod statedb_emit_log;
pub mod statedb_get_balance;
pub mod statedb_get_code;
pub mod statedb_get_code_hash;
pub mod statedb_get_code_size;
pub mod statedb_get_storage;
pub mod statedb_update_code;
pub mod statedb_update_storage;
pub mod sys_exec;
pub mod sys_halt;
pub mod sys_input_size;
pub mod sys_output_size;
pub mod sys_read;
pub mod sys_read_output;
pub mod sys_state;
pub mod sys_write;

use crate::{
    impl_runtime_handler,
    instruction::{
        crypto_ecrecover::CryptoEcrecover,
        crypto_keccak256::CryptoKeccak256,
        crypto_poseidon::CryptoPoseidon,
        crypto_poseidon2::CryptoPoseidon2,
        jzkt_checkpoint::JzktCheckpoint,
        jzkt_commit::JzktCommit,
        jzkt_compute_root::JzktComputeRoot,
        jzkt_emit_log::JzktEmitLog,
        jzkt_get::JzktGet,
        jzkt_load::JzktLoad,
        jzkt_open::JzktOpen,
        jzkt_preimage_copy::JzktPreimageCopy,
        jzkt_preimage_size::JzktPreimageSize,
        jzkt_remove::JzktRemove,
        jzkt_rollback::JzktRollback,
        jzkt_store::JzktStore,
        jzkt_update::JzktUpdate,
        rwasm_compile::RwasmCompile,
        rwasm_create::RwasmCreate,
        rwasm_transact::RwasmTransact,
        statedb_emit_log::StateDbEmitLog,
        statedb_get_balance::StateDbGetBalance,
        statedb_get_code::StateDbGetCode,
        statedb_get_code_hash::StateDbGetCodeHash,
        statedb_get_code_size::StateDbGetCodeSize,
        statedb_get_storage::StateDbGetStorage,
        statedb_update_code::StateDbUpdateCode,
        statedb_update_storage::StateDbUpdateStorage,
        sys_exec::SysExec,
        sys_halt::SysHalt,
        sys_input_size::SysInputSize,
        sys_output_size::SysOutputSize,
        sys_read::SysRead,
        sys_read_output::SysReadOutput,
        sys_state::SysState,
        sys_write::SysWrite,
    },
    runtime::RuntimeContext,
    types::{
        SysFuncIdx,
        SysFuncIdx::{
            CRYPTO_ECRECOVER,
            CRYPTO_KECCAK256,
            CRYPTO_POSEIDON,
            CRYPTO_POSEIDON2,
            JZKT_COMMIT,
            JZKT_COMPUTE_ROOT,
            JZKT_GET,
            JZKT_OPEN,
            JZKT_ROLLBACK,
            JZKT_UPDATE,
            RWASM_COMPILE,
            RWASM_CREATE,
            RWASM_TRANSACT,
            STATEDB_GET_STORAGE,
            STATEDB_UPDATE_STORAGE,
            SYS_HALT,
            SYS_INPUT_SIZE,
            SYS_READ,
            SYS_STATE,
            SYS_WRITE,
        },
    },
};
use fluentbase_types::SysFuncIdx::{
    JZKT_CHECKPOINT,
    JZKT_EMIT_LOG,
    JZKT_LOAD,
    JZKT_PREIMAGE_COPY,
    JZKT_PREIMAGE_SIZE,
    JZKT_REMOVE,
    JZKT_STORE,
    STATEDB_EMIT_LOG,
    STATEDB_GET_BALANCE,
    STATEDB_GET_CODE,
    STATEDB_GET_CODE_HASH,
    STATEDB_GET_CODE_SIZE,
    STATEDB_UPDATE_CODE,
    SYS_EXEC,
    SYS_OUTPUT_SIZE,
    SYS_READ_OUTPUT,
};
use rwasm_codegen::{
    rwasm::{Caller, Linker, Store},
    ImportLinker,
};

pub trait RuntimeHandler {
    const MODULE_NAME: &'static str;
    const FUNC_NAME: &'static str;
    const FUNC_INDEX: SysFuncIdx;

    fn register_linker<'t, T>(import_linker: &mut ImportLinker);
    fn register_handler<'t, T>(
        linker: &mut Linker<RuntimeContext<'t, T>>,
        store: &mut Store<RuntimeContext<'t, T>>,
    );
}

impl_runtime_handler!(CryptoKeccak256, CRYPTO_KECCAK256, fn fluentbase_v1alpha::_crypto_keccak256(data_offset: u32, data_len: u32, output_offset: u32) -> ());
impl_runtime_handler!(CryptoPoseidon, CRYPTO_POSEIDON, fn fluentbase_v1alpha::_crypto_poseidon(f32s_offset: u32, f32s_len: u32, output_offset: u32) -> ());
impl_runtime_handler!(CryptoPoseidon2, CRYPTO_POSEIDON2, fn fluentbase_v1alpha::_crypto_poseidon2(fa32_offset: u32, fb32_offset: u32, fd32_offset: u32, output_offset: u32) -> ());
impl_runtime_handler!(CryptoEcrecover, CRYPTO_ECRECOVER, fn fluentbase_v1alpha::_crypto_ecrecover(digest32_offset: u32, sig64_offset: u32, output65_offset: u32, rec_id: u32) -> ());

impl_runtime_handler!(SysHalt, SYS_HALT, fn fluentbase_v1alpha::_sys_halt(exit_code: i32) -> ());
impl_runtime_handler!(SysWrite, SYS_WRITE, fn fluentbase_v1alpha::_sys_write(offset: u32, length: u32) -> ());
impl_runtime_handler!(SysInputSize, SYS_INPUT_SIZE, fn fluentbase_v1alpha::_sys_input_size() -> u32);
impl_runtime_handler!(SysRead, SYS_READ, fn fluentbase_v1alpha::_sys_read(target: u32, offset: u32, length: u32) -> ());
impl_runtime_handler!(SysOutputSize, SYS_OUTPUT_SIZE, fn fluentbase_v1alpha::_sys_output_size() -> u32);
impl_runtime_handler!(SysReadOutput, SYS_READ_OUTPUT, fn fluentbase_v1alpha::_sys_read_output(target: u32, offset: u32, length: u32) -> ());
impl_runtime_handler!(SysState, SYS_STATE, fn fluentbase_v1alpha::_sys_state() -> u32);
impl_runtime_handler!(SysExec, SYS_EXEC, fn fluentbase_v1alpha::_sys_exec(code_offset: u32, code_len: u32, input_offset: u32, input_len: u32, return_offset: u32, return_len: u32, fuel_offset: u32, state: u32) -> i32);

impl_runtime_handler!(JzktOpen, JZKT_OPEN, fn fluentbase_v1alpha::_zktrie_open(root32_offset: u32) -> ());
impl_runtime_handler!(JzktCheckpoint, JZKT_CHECKPOINT, fn fluentbase_v1alpha::_jzkt_checkpoint() -> (u32, u32));
impl_runtime_handler!(JzktGet, JZKT_GET, fn fluentbase_v1alpha::_jzkt_get(key32_offset: u32, field: u32, output32_offset: u32) -> u32);
impl_runtime_handler!(JzktUpdate, JZKT_UPDATE, fn fluentbase_v1alpha::_jzkt_update(key32_offset: u32, flags: u32, vals32_offset: u32, vals32_len: u32) -> ());
impl_runtime_handler!(JzktRemove, JZKT_REMOVE, fn fluentbase_v1alpha::_jzkt_remove(key32_offset: u32) -> ());
impl_runtime_handler!(JzktComputeRoot, JZKT_COMPUTE_ROOT, fn fluentbase_v1alpha::_jzkt_compute_root(output32_offset: u32) -> ());
impl_runtime_handler!(JzktEmitLog, JZKT_EMIT_LOG, fn fluentbase_v1alpha::_jzkt_emit_log(key32_ptr: u32, topics32s_ptr: u32, topics32s_len: u32, data_ptr: u32, data_len: u32) -> ());
impl_runtime_handler!(JzktCommit, JZKT_COMMIT, fn fluentbase_v1alpha::_jzkt_commit(root32_offset: u32) -> ());
impl_runtime_handler!(JzktRollback, JZKT_ROLLBACK, fn fluentbase_v1alpha::_jzkt_rollback(checkpoint0: u32, checkpoint1: u32) -> ());
impl_runtime_handler!(JzktStore, JZKT_STORE, fn fluentbase_v1alpha::_jzkt_store(slot32_ptr: u32, value32_ptr: u32) -> ());
impl_runtime_handler!(JzktLoad, JZKT_LOAD, fn fluentbase_v1alpha::_jzkt_load(slot32_ptr: u32, value32_ptr: u32) -> i32);
impl_runtime_handler!(JzktPreimageSize, JZKT_PREIMAGE_SIZE, fn fluentbase_v1alpha::_jzkt_preimage_size(hash32_ptr: u32) -> u32);
impl_runtime_handler!(JzktPreimageCopy, JZKT_PREIMAGE_COPY, fn fluentbase_v1alpha::_jzkt_preimage_copy(hash32_ptr: u32, preimage_ptr: u32) -> ());

// TODO: "remove these impls"
impl_runtime_handler!(RwasmTransact, RWASM_TRANSACT, fn fluentbase_v1alpha::_rwasm_transact(address20_offset: u32, value32_offset: u32, input_offset: u32, input_length: u32, return_offset: u32, return_length: u32, fuel: u32, is_delegate: u32, is_static: u32) -> i32);
impl_runtime_handler!(RwasmCompile, RWASM_COMPILE, fn fluentbase_v1alpha::_rwasm_compile(input_offset: u32, input_len: u32, output_offset: u32, output_len: u32) -> i32);
impl_runtime_handler!(RwasmCreate, RWASM_CREATE, fn fluentbase_v1alpha::_rwasm_create(value32_offset: u32, input_bytecode_offset: u32, input_bytecode_length: u32, salt32_offset: u32, return_address20_offset: u32, is_create2: u32) -> i32);
impl_runtime_handler!(StateDbGetCode, STATEDB_GET_CODE, fn fluentbase_v1alpha::_statedb_get_code(key20_offset: u32, output_offset: u32, code_offset: u32, output_len: u32) -> ());
impl_runtime_handler!(StateDbGetCodeSize, STATEDB_GET_CODE_SIZE, fn fluentbase_v1alpha::_statedb_get_code_size(key20_offset: u32) -> u32);
impl_runtime_handler!(StateDbGetCodeHash, STATEDB_GET_CODE_HASH, fn fluentbase_v1alpha::_statedb_get_code_hash(key20_offset: u32, out_hash32_offset: u32) -> ());
impl_runtime_handler!(StateDbUpdateCode, STATEDB_UPDATE_CODE, fn fluentbase_v1alpha::_statedb_set_code(key20_offset: u32, code_offset: u32, code_len: u32) -> ());
impl_runtime_handler!(StateDbUpdateStorage, STATEDB_GET_STORAGE, fn fluentbase_v1alpha::_statedb_get_storage(key32_offset: u32, val32_offset: u32) -> ());
impl_runtime_handler!(StateDbGetStorage, STATEDB_UPDATE_STORAGE, fn fluentbase_v1alpha::_statedb_update_storage(key32_offset: u32, val32_offset: u32) -> ());
impl_runtime_handler!(StateDbEmitLog, STATEDB_EMIT_LOG, fn fluentbase_v1alpha::_statedb_emit_log(topics32_offset: u32, topics32_length: u32, data_offset: u32, data_len: u32) -> ());
impl_runtime_handler!(StateDbGetBalance, STATEDB_GET_BALANCE, fn fluentbase_v1alpha::_statedb_get_balance(address20_offset: u32, out_balance32_offset: u32, is_self: u32) -> ());

fn runtime_register_linkers<'t, T, const IS_SOVEREIGN: bool>(import_linker: &mut ImportLinker) {
    CryptoKeccak256::register_linker::<T>(import_linker);
    CryptoPoseidon::register_linker::<T>(import_linker);
    CryptoPoseidon2::register_linker::<T>(import_linker);
    CryptoEcrecover::register_linker::<T>(import_linker);
    SysHalt::register_linker::<T>(import_linker);
    SysWrite::register_linker::<T>(import_linker);
    SysInputSize::register_linker::<T>(import_linker);
    SysRead::register_linker::<T>(import_linker);
    SysOutputSize::register_linker::<T>(import_linker);
    SysReadOutput::register_linker::<T>(import_linker);
    SysExec::register_linker::<T>(import_linker);
    SysState::register_linker::<T>(import_linker);
    if IS_SOVEREIGN {
        JzktOpen::register_linker::<T>(import_linker);
        JzktCheckpoint::register_linker::<T>(import_linker);
        JzktGet::register_linker::<T>(import_linker);
        JzktUpdate::register_linker::<T>(import_linker);
        JzktRemove::register_linker::<T>(import_linker);
        JzktComputeRoot::register_linker::<T>(import_linker);
    }
    JzktEmitLog::register_linker::<T>(import_linker);
    if IS_SOVEREIGN {
        JzktCommit::register_linker::<T>(import_linker);
        JzktRollback::register_linker::<T>(import_linker);
    }
    JzktStore::register_linker::<T>(import_linker);
    JzktLoad::register_linker::<T>(import_linker);
    if IS_SOVEREIGN {
        // PreimageSize::register_linker::<T>(import_linker);
        // PreimageCopy::register_linker::<T>(import_linker);
    }
    RwasmTransact::register_linker::<T>(import_linker);
    RwasmCompile::register_linker::<T>(import_linker);
    RwasmCreate::register_linker::<T>(import_linker);
    StateDbGetCode::register_linker::<T>(import_linker);
    StateDbGetCodeSize::register_linker::<T>(import_linker);
    StateDbGetCodeHash::register_linker::<T>(import_linker);
    StateDbUpdateCode::register_linker::<T>(import_linker);
    StateDbUpdateStorage::register_linker::<T>(import_linker);
    StateDbGetStorage::register_linker::<T>(import_linker);
    StateDbEmitLog::register_linker::<T>(import_linker);
    StateDbGetBalance::register_linker::<T>(import_linker);
}

pub(crate) fn runtime_register_sovereign_linkers<'t, T>(import_linker: &mut ImportLinker) {
    runtime_register_linkers::<T, true>(import_linker);
}

pub(crate) fn runtime_register_shared_linkers<'t, T>(import_linker: &mut ImportLinker) {
    runtime_register_linkers::<T, false>(import_linker);
}

fn runtime_register_handlers<'t, T, const IS_SOVEREIGN: bool>(
    linker: &mut Linker<RuntimeContext<'t, T>>,
    store: &mut Store<RuntimeContext<'t, T>>,
) {
    CryptoKeccak256::register_handler(linker, store);
    CryptoPoseidon::register_handler(linker, store);
    CryptoPoseidon2::register_handler(linker, store);
    CryptoEcrecover::register_handler(linker, store);
    SysHalt::register_handler(linker, store);
    SysWrite::register_handler(linker, store);
    SysInputSize::register_handler(linker, store);
    SysRead::register_handler(linker, store);
    SysOutputSize::register_handler(linker, store);
    SysReadOutput::register_handler(linker, store);
    SysExec::register_handler(linker, store);
    SysState::register_handler(linker, store);
    if IS_SOVEREIGN {
        JzktOpen::register_handler(linker, store);
        JzktCheckpoint::register_handler(linker, store);
        JzktGet::register_handler(linker, store);
        JzktUpdate::register_handler(linker, store);
        JzktRemove::register_handler(linker, store);
        JzktComputeRoot::register_handler(linker, store);
    }
    JzktEmitLog::register_handler(linker, store);
    if IS_SOVEREIGN {
        JzktCommit::register_handler(linker, store);
        JzktRollback::register_handler(linker, store);
    }
    JzktStore::register_handler(linker, store);
    JzktLoad::register_handler(linker, store);
    if IS_SOVEREIGN {
        // PreimageSize::register_handler(linker, store);
        // PreimageCopy::register_handler(linker, store);
    }
    RwasmTransact::register_handler(linker, store);
    RwasmCompile::register_handler(linker, store);
    RwasmCreate::register_handler(linker, store);
    StateDbGetCode::register_handler(linker, store);
    StateDbGetCodeSize::register_handler(linker, store);
    StateDbGetCodeHash::register_handler(linker, store);
    StateDbUpdateCode::register_handler(linker, store);
    StateDbUpdateStorage::register_handler(linker, store);
    StateDbGetStorage::register_handler(linker, store);
    StateDbEmitLog::register_handler(linker, store);
    StateDbGetBalance::register_handler(linker, store);
}

pub fn runtime_register_sovereign_handlers<'t, T>(
    linker: &mut Linker<RuntimeContext<'t, T>>,
    store: &mut Store<RuntimeContext<'t, T>>,
) {
    runtime_register_handlers::<T, true>(linker, store);
}

pub fn runtime_register_shared_handlers<'t, T>(
    linker: &mut Linker<RuntimeContext<'t, T>>,
    store: &mut Store<RuntimeContext<'t, T>>,
) {
    runtime_register_handlers::<T, false>(linker, store);
}
