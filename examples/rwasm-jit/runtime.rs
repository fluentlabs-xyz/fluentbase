// pub mod charge_fuel;
// pub mod checkpoint;
// pub mod commit;
// pub mod compute_root;
// pub mod debug_log;
// pub mod ecrecover;
// pub mod emit_log;
// pub mod exec;
pub mod exit;
// pub mod forward_output;
// pub mod fuel;
// pub mod get_leaf;
pub mod input_size;
pub mod keccak256;
// pub mod output_size;
// pub mod poseidon;
// pub mod poseidon_hash;
// pub mod preimage_copy;
// pub mod preimage_size;
pub mod read;
// pub mod read_context;
// pub mod read_output;
// pub mod resume;
// pub mod rollback;
// pub mod state;
// pub mod update_leaf;
// pub mod update_preimage;
pub mod write;

use crate::runtime::{
    // charge_fuel::SyscallChargeFuel,
    // debug_log::SyscallDebugLog,
    // ecrecover::SyscallEcrecover,
    // exec::SyscallExec,
    exit::SyscallExit,
    // forward_output::SyscallForwardOutput,
    // fuel::SyscallFuel,
    input_size::SyscallInputSize,
    keccak256::SyscallKeccak256,
    // output_size::SyscallOutputSize,
    // poseidon::SyscallPoseidon,
    // poseidon_hash::SyscallPoseidonHash,
    // preimage_copy::SyscallPreimageCopy,
    // preimage_size::SyscallPreimageSize,
    read::SyscallRead,
    // read_context::SyscallReadContext,
    // read_output::SyscallReadOutput,
    // resume::SyscallResume,
    // state::SyscallState,
    write::SyscallWrite,
};
use fluentbase_sdk::{Bytes, SysFuncIdx, F254, POSEIDON_EMPTY};
use rwasm::{Caller, Linker, ResumableInvocation, Store};

extern crate alloc;

use crate::impl_runtime_handler;
use alloc::vec::Vec;

pub struct RuntimeContext {
    // context inputs
    pub(crate) bytecode: BytecodeOrHash,
    pub(crate) fuel_limit: u64,
    pub(crate) state: u32,
    pub(crate) call_depth: u32,
    pub(crate) trace: bool,
    // #[deprecated(note = "this parameter can be removed, we filter on the AOT level")]
    pub(crate) is_shared: bool,
    pub(crate) input: Vec<u8>,
    pub(crate) context: Vec<u8>,
    // context outputs
    pub(crate) execution_result: ExecutionResult,
    pub(crate) resumable_invocation: Option<ResumableInvocation>,
    // pub(crate) jzkt: Option<Box<dyn IJournaledTrie>>,
}

impl Default for RuntimeContext {
    fn default() -> Self {
        Self {
            bytecode: BytecodeOrHash::default(),
            fuel_limit: 0,
            state: 0,
            is_shared: false,
            input: Vec::new(),
            context: Vec::new(),
            call_depth: 0,
            trace: false,
            execution_result: ExecutionResult::default(),
            resumable_invocation: None,
            // jzkt: None,
        }
    }
}

impl RuntimeContext {
    pub fn new<I: Into<Bytes>>(bytecode: I) -> Self {
        Self {
            bytecode: BytecodeOrHash::Bytecode(bytecode.into(), None),
            ..Default::default()
        }
    }

    pub fn new_with_hash(bytecode_hash: F254) -> Self {
        Self {
            bytecode: BytecodeOrHash::Hash(bytecode_hash),
            ..Default::default()
        }
    }

    pub fn with_input(mut self, input_data: Vec<u8>) -> Self {
        self.input = input_data;
        self
    }

    pub fn with_context(mut self, context: Vec<u8>) -> Self {
        self.context = context;
        self
    }

    pub fn change_input(&mut self, input_data: Vec<u8>) {
        self.input = input_data;
    }

    pub fn change_context(&mut self, new_context: Vec<u8>) {
        self.context = new_context;
    }

    pub fn with_state(mut self, state: u32) -> Self {
        self.state = state;
        self
    }

    pub fn with_is_shared(mut self, is_shared: bool) -> Self {
        self.is_shared = is_shared;
        self
    }

    pub fn with_fuel_limit(mut self, fuel_limit: u64) -> Self {
        self.fuel_limit = fuel_limit;
        self
    }

    // pub fn with_jzkt(mut self, jzkt: Box<dyn IJournaledTrie>) -> Self {
    //     self.jzkt = Some(jzkt);
    //     self
    // }

    pub fn with_depth(mut self, depth: u32) -> Self {
        self.call_depth = depth;
        self
    }

    pub fn with_tracer(mut self) -> Self {
        self.trace = true;
        self
    }

    // pub fn jzkt(&self) -> &Box<dyn IJournaledTrie> {
    //     self.jzkt.as_ref().expect("jzkt is not initialized")
    // }

    pub fn depth(&self) -> u32 {
        self.call_depth
    }

    pub fn exit_code(&self) -> i32 {
        self.execution_result.exit_code
    }

    pub fn input(&self) -> &Vec<u8> {
        self.input.as_ref()
    }

    pub fn input_size(&self) -> u32 {
        self.input.len() as u32
    }

    pub fn context(&self) -> &Vec<u8> {
        self.context.as_ref()
    }

    pub fn context_size(&self) -> u32 {
        self.context.len() as u32
    }

    pub fn argv_buffer(&self) -> Vec<u8> {
        self.input().clone()
    }

    pub fn output(&self) -> &Vec<u8> {
        &self.execution_result.output
    }

    pub fn output_mut(&mut self) -> &mut Vec<u8> {
        &mut self.execution_result.output
    }

    pub fn fuel_limit(&self) -> u64 {
        self.fuel_limit
    }

    pub fn return_data(&self) -> &Vec<u8> {
        &self.execution_result.return_data
    }

    pub fn return_data_mut(&mut self) -> &mut Vec<u8> {
        &mut self.execution_result.return_data
    }

    pub fn state(&self) -> u32 {
        self.state
    }

    pub fn clear_output(&mut self) {
        self.execution_result.output.clear();
    }
}

#[derive(Clone)]
pub enum BytecodeOrHash {
    Bytecode(Bytes, Option<F254>),
    Hash(F254),
}
impl Default for BytecodeOrHash {
    fn default() -> Self {
        Self::Bytecode(Bytes::new(), Some(POSEIDON_EMPTY))
    }
}

#[derive(Default, Clone, Debug)]
pub struct ExecutionResult {
    pub exit_code: i32,
    pub fuel_consumed: u64,
    pub return_data: Vec<u8>,
    pub output: Vec<u8>,
    pub interrupted: bool,
}

pub trait RuntimeHandler {
    const MODULE_NAME: &'static str;
    const FUNC_NAME: &'static str;
    const FUNC_INDEX: SysFuncIdx;

    fn register_handler(linker: &mut Linker<RuntimeContext>, store: &mut Store<RuntimeContext>);
}

pub fn runtime_register_sovereign_handlers(
    linker: &mut Linker<RuntimeContext>,
    store: &mut Store<RuntimeContext>,
) {
    SyscallKeccak256::register_handler(linker, store);
    // SyscallPoseidon::register_handler(linker, store);
    // SyscallPoseidonHash::register_handler(linker, store);
    // SyscallEcrecover::register_handler(linker, store);
    SyscallExit::register_handler(linker, store);
    // SyscallState::register_handler(linker, store);
    SyscallRead::register_handler(linker, store);
    SyscallInputSize::register_handler(linker, store);
    SyscallWrite::register_handler(linker, store);
    // SyscallOutputSize::register_handler(linker, store);
    // SyscallReadOutput::register_handler(linker, store);
    // SyscallExec::register_handler(linker, store);
    // SyscallResume::register_handler(linker, store);
    // SyscallForwardOutput::register_handler(linker, store);
    // SyscallChargeFuel::register_handler(linker, store);
    // SyscallFuel::register_handler(linker, store);
    // SyscallReadContext::register_handler(linker, store);
    // SyscallPreimageSize::register_handler(linker, store);
    // SyscallPreimageCopy::register_handler(linker, store);
    // SyscallDebugLog::register_handler(linker, store);
}

impl_runtime_handler!(SyscallKeccak256, KECCAK256, fn fluentbase_v1preview::_keccak256(data_ptr: u32, data_len: u32, output_ptr: u32) -> ());
// impl_runtime_handler!(SyscallPoseidon, POSEIDON, fn fluentbase_v1preview::_poseidon(f32s_ptr:
// u32, f32s_len: u32, output_ptr: u32) -> ()); impl_runtime_handler!(SyscallPoseidonHash,
// POSEIDON_HASH, fn fluentbase_v1preview::_poseidon_hash(fa32_ptr: u32, fb32_ptr: u32, fd32_ptr:
// u32, output_ptr: u32) -> ()); impl_runtime_handler!(SyscallEcrecover, ECRECOVER, fn
// fluentbase_v1preview::_ecrecover(digest32_ptr: u32, sig64_ptr: u32, output65_ptr: u32, rec_id:
// u32) -> ());
impl_runtime_handler!(SyscallExit, EXIT, fn fluentbase_v1preview::_exit(exit_code: i32) -> ());
// impl_runtime_handler!(SyscallState, STATE, fn fluentbase_v1preview::_state() -> u32);
impl_runtime_handler!(SyscallRead, READ_INPUT, fn fluentbase_v1preview::_read(target: u32, offset: u32, length: u32) -> ());
impl_runtime_handler!(SyscallInputSize, INPUT_SIZE, fn fluentbase_v1preview::_input_size() -> u32);
impl_runtime_handler!(SyscallWrite, WRITE_OUTPUT, fn fluentbase_v1preview::_write(offset: u32, length: u32) -> ());
// impl_runtime_handler!(SyscallOutputSize, OUTPUT_SIZE, fn fluentbase_v1preview::_output_size() ->
// u32); impl_runtime_handler!(SyscallReadOutput, READ_OUTPUT, fn
// fluentbase_v1preview::_read_output(target: u32, offset: u32, length: u32) -> ());
// impl_runtime_handler!(SyscallExec, EXEC, fn fluentbase_v1preview::_exec(code_hash32_ptr: u32,
// input_ptr: u32, input_len: u32, fuel_ptr: u32, state: u32) -> i32); impl_runtime_handler!
// (SyscallResume, RESUME, fn fluentbase_v1preview::_resume(call_id: u32, return_data_ptr: u32,
// return_data_len: u32, exit_code: i32, fuel_ptr: u32) -> i32); impl_runtime_handler!
// (SyscallForwardOutput, FORWARD_OUTPUT, fn fluentbase_v1preview::_forward_output(offset: u32, len:
// u32) -> ()); impl_runtime_handler!(SyscallChargeFuel, CHARGE_FUEL, fn
// fluentbase_v1preview::_charge_fuel(delta: u64) -> u64); impl_runtime_handler!(SyscallFuel, FUEL,
// fn fluentbase_v1preview::_fuel() -> u64); impl_runtime_handler!(SyscallReadContext, READ_CONTEXT,
// fn fluentbase_v1preview::_read_context(target_ptr: u32, offset: u32, length: u32) -> ());
// impl_runtime_handler!(SyscallPreimageSize, PREIMAGE_SIZE, fn
// fluentbase_v1preview::_preimage_size(hash32_ptr: u32) -> u32); impl_runtime_handler!
// (SyscallPreimageCopy, PREIMAGE_COPY, fn fluentbase_v1preview::_preimage_copy(hash32_ptr: u32,
// preimage_ptr: u32) -> ()); impl_runtime_handler!(SyscallDebugLog, DEBUG_LOG, fn
// fluentbase_v1preview::_debug_log(msg_ptr: u32, msg_len: u32) -> ());
