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

use crate::{
    impl_runtime_handler,
    instruction::{
        crypto_ecrecover::CryptoEcrecover,
        crypto_keccak256::CryptoKeccak256,
        crypto_poseidon::CryptoPoseidon,
        crypto_poseidon2::CryptoPoseidon2,
        rwasm_compile::RwasmCompile,
        rwasm_transact::RwasmTransact,
        sys_halt::SysHalt,
        sys_input_size::SysInputSize,
        sys_read::SysRead,
        sys_state::SysState,
        sys_write::SysWrite,
    },
    runtime::RuntimeContext,
    SysFuncIdx,
    SysFuncIdx::{
        CRYPTO_ECRECOVER,
        CRYPTO_KECCAK256,
        CRYPTO_POSEIDON,
        CRYPTO_POSEIDON2,
        RWASM_COMPILE,
        RWASM_TRANSACT,
        SYS_HALT,
        SYS_INPUT_SIZE,
        SYS_READ,
        SYS_STATE,
        SYS_WRITE,
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

impl_runtime_handler!(CryptoKeccak256, CRYPTO_KECCAK256, fn fluentbase_v1alpha::_crypto_keccak256(data_offset: u32, data_len: u32, output_offset: u32) -> ());
impl_runtime_handler!(CryptoPoseidon, CRYPTO_POSEIDON, fn fluentbase_v1alpha::_crypto_poseidon(f32s_offset: u32, f32s_len: u32, output_offset: u32) -> ());
impl_runtime_handler!(CryptoPoseidon2, CRYPTO_POSEIDON2, fn fluentbase_v1alpha::_crypto_poseidon2(fa32_offset: u32, fb32_offset: u32, fd32_offset: u32, output_offset: u32) -> ());
impl_runtime_handler!(CryptoEcrecover, CRYPTO_ECRECOVER, fn fluentbase_v1alpha::_crypto_ecrecover(digest32_offset: u32, sig64_offset: u32, output65_offset: u32, rec_id: u32) -> ());

impl_runtime_handler!(RwasmCompile, RWASM_COMPILE, fn fluentbase_v1alpha::_rwasm_compile(input_offset: u32, input_len: u32, output_offset: u32, output_len: u32) -> i32);
impl_runtime_handler!(RwasmTransact, RWASM_TRANSACT, fn fluentbase_v1alpha::_rwasm_transact(code_offset: u32, code_len: u32, input_offset: u32, input_len: u32, output_offset: u32, output_len: u32, state: u32, fuel: u32) -> i32);

pub(crate) fn runtime_register_linkers<'t, T>(import_linker: &mut ImportLinker) {
    CryptoKeccak256::register_linker::<T>(import_linker);
    CryptoPoseidon::register_linker::<T>(import_linker);
    CryptoPoseidon2::register_linker::<T>(import_linker);
    CryptoEcrecover::register_linker::<T>(import_linker);
    RwasmCompile::register_linker::<T>(import_linker);
    RwasmTransact::register_linker::<T>(import_linker);
    SysHalt::register_linker::<T>(import_linker);
    SysState::register_linker::<T>(import_linker);
    SysRead::register_linker::<T>(import_linker);
    SysInputSize::register_linker::<T>(import_linker);
    SysWrite::register_linker::<T>(import_linker);
}

pub(crate) fn runtime_register_handlers<'t, T>(
    linker: &mut Linker<RuntimeContext<'t, T>>,
    store: &mut Store<RuntimeContext<'t, T>>,
) {
    CryptoKeccak256::register_handler(linker, store);
    CryptoPoseidon::register_handler(linker, store);
    CryptoPoseidon2::register_handler(linker, store);
    CryptoEcrecover::register_handler(linker, store);
    RwasmCompile::register_handler(linker, store);
    RwasmTransact::register_handler(linker, store);
    SysHalt::register_handler(linker, store);
    SysState::register_handler(linker, store);
    SysRead::register_handler(linker, store);
    SysInputSize::register_handler(linker, store);
    SysWrite::register_handler(linker, store);
}
