pub mod crypto_ecrecover;
pub mod crypto_keccak256;
pub mod crypto_poseidon;
pub mod crypto_poseidon2;
pub mod rwasm_compile;
pub mod rwasm_transact;
pub mod sys_halt;
pub mod sys_input_size;
pub mod sys_read;
pub mod sys_state;
pub mod sys_write;
pub mod zktrie_commit;
pub mod zktrie_field;
pub mod zktrie_load;
pub mod zktrie_open;
pub mod zktrie_rollback;
pub mod zktrie_root;
pub mod zktrie_store;
pub mod zktrie_update;

use crate::{
    impl_runtime_handler,
    instruction::{
        crypto_ecrecover::SysEcrecover,
        crypto_keccak256::SysKeccak256,
        crypto_poseidon::SysPoseidon,
        crypto_poseidon2::SysPoseidon2,
        rwasm_compile::SysCompile,
        rwasm_transact::SysExec,
        sys_halt::SysHalt,
        sys_input_size::SysInputSize,
        sys_read::SysRead,
        sys_state::SysState,
        sys_write::SysWrite,
        zktrie_commit::ZkTrieCommit,
        zktrie_field::ZkTrieField,
        zktrie_load::ZkTrieLoad,
        zktrie_open::ZkTrieOpen,
        zktrie_rollback::ZkTrieRollback,
        zktrie_root::ZkTrieRoot,
        zktrie_store::ZkTrieStore,
        zktrie_update::ZkTrieUpdate,
    },
    runtime::RuntimeContext,
    types::{
        SysFuncIdx,
        SysFuncIdx::{
            CRYPTO_ECRECOVER,
            CRYPTO_KECCAK256,
            CRYPTO_POSEIDON,
            CRYPTO_POSEIDON2,
            SYS_COMPILE,
            SYS_EXEC,
            SYS_HALT,
            SYS_INPUT_SIZE,
            SYS_READ,
            SYS_STATE,
            SYS_WRITE,
            ZKTRIE_COMMIT,
            ZKTRIE_FIELD,
            ZKTRIE_LOAD,
            ZKTRIE_OPEN,
            ZKTRIE_ROLLBACK,
            ZKTRIE_ROOT,
            ZKTRIE_STORE,
            ZKTRIE_UPDATE,
        },
    },
};
use fluentbase_rwasm::{rwasm::ImportLinker, Caller, Linker, Store};

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

impl_runtime_handler!(SysHalt, SYS_HALT, fn fluentbase_v1alpha::_sys_halt(exit_code: i32) -> ());
impl_runtime_handler!(SysState, SYS_STATE, fn fluentbase_v1alpha::_sys_state() -> u32);
impl_runtime_handler!(SysRead, SYS_READ, fn fluentbase_v1alpha::_sys_read(target: u32, offset: u32, length: u32) -> ());
impl_runtime_handler!(SysInputSize, SYS_INPUT_SIZE, fn fluentbase_v1alpha::_sys_input_size() -> u32);
impl_runtime_handler!(SysWrite, SYS_WRITE, fn fluentbase_v1alpha::_sys_write(offset: u32, length: u32) -> ());
impl_runtime_handler!(SysKeccak256, CRYPTO_KECCAK256, fn fluentbase_v1alpha::_crypto_keccak256(data_offset: u32, data_len: u32, output_offset: u32) -> ());
impl_runtime_handler!(SysPoseidon, CRYPTO_POSEIDON, fn fluentbase_v1alpha::_crypto_poseidon(f32s_offset: u32, f32s_len: u32, output_offset: u32) -> ());
impl_runtime_handler!(SysPoseidon2, CRYPTO_POSEIDON2, fn fluentbase_v1alpha::_crypto_poseidon2(fa32_offset: u32, fb32_offset: u32, fd32_offset: u32, output_offset: u32) -> ());
impl_runtime_handler!(SysEcrecover, CRYPTO_ECRECOVER, fn fluentbase_v1alpha::_crypto_ecrecover(digest32_offset: u32, sig64_offset: u32, output65_offset: u32, rec_id: u32) -> ());
impl_runtime_handler!(SysExec, SYS_EXEC, fn fluentbase_v1alpha::_rwasm_transact(code_offset: u32, code_len: u32, input_offset: u32, input_len: u32, output_offset: u32, output_len: u32, state: u32, fuel: u32) -> i32);
impl_runtime_handler!(SysCompile, SYS_COMPILE, fn fluentbase_v1alpha::_rwasm_compile(input_offset: u32, input_len: u32, output_offset: u32, output_len: u32) -> i32);
impl_runtime_handler!(ZkTrieOpen, ZKTRIE_OPEN, fn fluentbase_v1alpha::_zktrie_open(root32_offset: u32) -> ());
impl_runtime_handler!(ZkTrieUpdate, ZKTRIE_UPDATE, fn fluentbase_v1alpha::_zktrie_update(key32_offset: u32, flags: u32, vals32_offset: u32, vals32_len: u32) -> ());
impl_runtime_handler!(ZkTrieField, ZKTRIE_FIELD, fn fluentbase_v1alpha::_zktrie_field(key32_offset: u32, field: u32, output32_offset: u32) -> ());
impl_runtime_handler!(ZkTrieRoot, ZKTRIE_ROOT, fn fluentbase_v1alpha::_zktrie_root(output32_offset: u32) -> ());
impl_runtime_handler!(ZkTrieRollback, ZKTRIE_ROLLBACK, fn fluentbase_v1alpha::_zktrie_rollback() -> ());
impl_runtime_handler!(ZkTrieCommit, ZKTRIE_COMMIT, fn fluentbase_v1alpha::_zktrie_commit() -> ());
impl_runtime_handler!(ZkTrieStore, ZKTRIE_STORE, fn fluentbase_v1alpha::_zktrie_store(key32_offset: u32, val32_offset: u32) -> ());
impl_runtime_handler!(ZkTrieLoad, ZKTRIE_LOAD, fn fluentbase_v1alpha::_zktrie_load(key32_offset: u32, val32_offset: u32) -> ());

pub(crate) fn runtime_register_linkers<'t, T>(import_linker: &mut ImportLinker) {
    SysHalt::register_linker::<T>(import_linker);
    SysState::register_linker::<T>(import_linker);
    SysRead::register_linker::<T>(import_linker);
    SysInputSize::register_linker::<T>(import_linker);
    SysWrite::register_linker::<T>(import_linker);
    SysKeccak256::register_linker::<T>(import_linker);
    SysPoseidon::register_linker::<T>(import_linker);
    SysPoseidon2::register_linker::<T>(import_linker);
    SysEcrecover::register_linker::<T>(import_linker);
    SysExec::register_linker::<T>(import_linker);
    SysCompile::register_linker::<T>(import_linker);
    ZkTrieOpen::register_linker::<T>(import_linker);
    ZkTrieUpdate::register_linker::<T>(import_linker);
    ZkTrieField::register_linker::<T>(import_linker);
    ZkTrieRoot::register_linker::<T>(import_linker);
    ZkTrieRollback::register_linker::<T>(import_linker);
    ZkTrieCommit::register_linker::<T>(import_linker);
    ZkTrieStore::register_linker::<T>(import_linker);
    ZkTrieLoad::register_linker::<T>(import_linker);
}

pub(crate) fn runtime_register_handlers<'t, T>(
    linker: &mut Linker<RuntimeContext<'t, T>>,
    store: &mut Store<RuntimeContext<'t, T>>,
) {
    SysHalt::register_handler(linker, store);
    SysState::register_handler(linker, store);
    SysRead::register_handler(linker, store);
    SysInputSize::register_handler(linker, store);
    SysWrite::register_handler(linker, store);
    SysKeccak256::register_handler(linker, store);
    SysPoseidon::register_handler(linker, store);
    SysPoseidon2::register_handler(linker, store);
    SysEcrecover::register_handler(linker, store);
    SysExec::register_handler(linker, store);
    SysCompile::register_handler(linker, store);
    ZkTrieOpen::register_handler(linker, store);
    ZkTrieUpdate::register_handler(linker, store);
    ZkTrieField::register_handler(linker, store);
    ZkTrieRoot::register_handler(linker, store);
    ZkTrieRollback::register_handler(linker, store);
    ZkTrieCommit::register_handler(linker, store);
    ZkTrieStore::register_handler(linker, store);
    ZkTrieLoad::register_handler(linker, store);
}
