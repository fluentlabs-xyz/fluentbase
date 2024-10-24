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
    },
    RuntimeContext,
};
use fluentbase_types::SysFuncIdx;
use rwasm::{Caller, Linker, Store};

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
}
