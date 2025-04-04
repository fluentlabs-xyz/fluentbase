use crate::{
    instruction::{debug_log::SyscallDebugLog, keccak256::SyscallKeccak256},
    RuntimeContext,
};
use anyhow::Result;
use core::{error::Error as StdError, fmt};
use fluentbase_codec::{bytes::BytesMut, CompactABI};
use fluentbase_types::{Bytes, ExitCode, FixedBytes, SyscallInvocationParams, B256, U256};

use fluentbase_types::{
    byteorder::{ByteOrder, LittleEndian}};
use std::{
    cell::RefCell,
    collections::{
        hash_map::{
            DefaultHasher,
            Entry::{Occupied, Vacant},
        },
        HashMap,
    },
    future::Future,
    hash::{Hash, Hasher},
    mem::drop,
    pin::Pin,
    sync::mpsc,
    task::{Context, Poll, RawWaker, RawWakerVTable, Waker},
    thread,
    thread::JoinHandle,
    time::Instant,
};
use wasmtime::{Caller, Config, Engine, Extern, Func, Linker, Memory, Module, Store};

struct WorkerContext {
    sender: mpsc::Sender<Message>,
    receiver: mpsc::Receiver<Message>,
    input: Vec<u8>,
    output: Vec<u8>,
    return_data: Vec<u8>,
}

#[derive(Debug, Clone)]
enum TerminationReason {
    Exit(i32),
    InputOutputOutOfBounds,
    Trap(wasmtime::Trap),
}

impl fmt::Display for TerminationReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl StdError for TerminationReason {}

#[derive(Debug)]
enum Message {
    ExecutionRequest {
        code_hash: [u8; 32],
        input: Vec<u8>,
        fuel_limit: u64,
        state: u32,
        fuel16_ptr: u32,
    },
    ExecutionResult {
        fuel_consumed: u64,
        fuel_refunded: i64,
        exit_code: i32,
        output: Vec<u8>,
        fuel16_ptr: u32,
    },
    WasmTerminated {
        reason: TerminationReason,
        output: Vec<u8>,
    },
}
struct AsyncExecutor {
    sender: mpsc::Sender<Message>,
    receiver: mpsc::Receiver<Message>,
    worker: JoinHandle<()>,
}

impl AsyncExecutor {
    /* module is expected to be preferified somewhere before.
     * So in this function we should never get errors during initialization of wasm instance */
    pub fn launch(module: &Module, input: Vec<u8>) -> Self {
        let (executor_sender, worker_receiver) = mpsc::channel();
        let (worker_sender, executor_receiver) = mpsc::channel();
        let worker_context = WorkerContext {
            sender: worker_sender,
            receiver: worker_receiver,
            input: input,
            output: Vec::new(),
            return_data: Vec::new(),
        };
        let engine = module.engine();
        let linker = new_linker_with_builtins(engine).unwrap();
        let mut store = Store::new(engine, worker_context);

        let instance = linker
            .instantiate(&mut store, &module)
            .expect("bytecode should be preverified");
        let main = instance
            .get_typed_func::<(), ()>(&mut store, "main")
            .expect("bytecode should be preferified");

        let worker = thread::spawn(move || {
            let reason = match main.call(&mut store, ()) {
                Ok(()) => TerminationReason::Exit(0),
                Err(e) => {
                    if let Some(reason) = e.downcast_ref::<TerminationReason>() {
                        reason.clone()
                    } else if let Some(trap) = e.downcast_ref::<wasmtime::Trap>() {
                        TerminationReason::Trap(trap.clone())
                    } else {
                        panic!("unexpected error occured during wasm guest execution");
                    }
                }
            };
            let message = Message::WasmTerminated {
                reason,
                output: store.data().output.clone(),
            };
            store
                .data()
                .sender
                .send(message)
                .expect("failed to send termination info to channel");
        });

        Self {
            sender: executor_sender,
            receiver: executor_receiver,
            worker,
        }
    }
    pub fn receive_message(&self) -> Message {
        self.receiver.recv().expect("maybe worker is dead")
    }
    pub fn send_message(&self, message: Message) -> () {
        self.sender.send(message).expect("maybe worker is dead")
    }
}

thread_local! {
    static RUNTIME_STATE: RefCell<RuntimeState> = RefCell::new(RuntimeState::new());
}
struct RuntimeState {
    modules_cache: HashMap<u64, Module>,
    suspended_executors: HashMap<i32, AsyncExecutor>,
    prev_call_id: i32,
}

impl RuntimeState {
    fn new() -> Self {
        Self {
            modules_cache: HashMap::new(),
            suspended_executors: HashMap::new(),
            prev_call_id: 0,
        }
    }

    fn suspend_executor(&mut self, executor: AsyncExecutor) -> i32 {
        if self.prev_call_id == i32::MAX {
            self.prev_call_id = 0; // wrap value around to prevent non-positive call ids
        }
        let call_id = self.prev_call_id + 1;
        self.prev_call_id = call_id;
        let existing = self.suspended_executors.insert(call_id, executor);
        assert!(existing.is_none());
        call_id
    }

    fn init_module_cached(&mut self, wasm_bytecode: &[u8]) -> &Module {
        let mut hasher = DefaultHasher::new();
        wasm_bytecode.hash(&mut hasher);
        let hash = hasher.finish();

        if !self.modules_cache.contains_key(&hash) {
            let config = Config::new();
            let engine = Engine::new(&config).unwrap();
            let module =
                Module::new(&engine, wasm_bytecode).expect("bytecode should be preverified");
            self.modules_cache.insert(hash, module);
        }

        self.modules_cache.get(&hash).unwrap()
    }

    fn take_suspended_executor(&mut self, call_id: i32) -> AsyncExecutor {
        let executor = self.suspended_executors.remove(&call_id);
        executor.expect("executor should exist")
    }
}

mod builtins {
    use super::*;
    fn get_memory_export(caller: &mut Caller<'_, WorkerContext>) -> anyhow::Result<Memory> {
        match caller.get_export("memory") {
            Some(Extern::Memory(memory)) => Ok(memory),
            _ => Err(wasmtime::Trap::MemoryOutOfBounds.into()),
        }
    }

    fn write_memory(
        caller: &mut Caller<'_, WorkerContext>,
        offset: u32,
        buffer: &[u8],
    ) -> anyhow::Result<()> {
        let memory = get_memory_export(caller)?;
        memory
            .write(caller, offset as usize, &buffer)
            .map_err(|_| wasmtime::Trap::MemoryOutOfBounds)?;
        Ok(())
    }

    fn read_memory(
        caller: &mut Caller<'_, WorkerContext>,
        offset: u32,
        length: u32,
    ) -> anyhow::Result<Vec<u8>> {
        let memory = get_memory_export(caller)?;
        let data = memory
            .data(&caller)
            .get(offset as usize..)
            .and_then(|arr| arr.get(..length as usize));
        if data.is_none() {
            Err(wasmtime::Trap::MemoryOutOfBounds.into())
        } else {
            Ok(Vec::from(data.unwrap()))
        }
    }

    pub fn write(
        mut caller: Caller<'_, WorkerContext>,
        offset: u32,
        length: u32,
    ) -> anyhow::Result<()> {
        let data = read_memory(&mut caller, offset, length)?;
        let context = caller.data_mut();
        println!("builtins::write(offset={}, length={})", offset, length);
        context.output.extend_from_slice(&data);
        Ok(())
    }

    pub fn read(
        mut caller: Caller<'_, WorkerContext>,
        target_ptr: u32,
        offset: u32,
        length: u32,
    ) -> anyhow::Result<()> {
        let context = caller.data();
        if offset + length <= context.input.len() as u32 {
            let buffer =
                context.input[(offset as usize)..(offset as usize + length as usize)].to_vec();
            write_memory(&mut caller, target_ptr, &buffer)?;
            Ok(())
        } else {
            Err(TerminationReason::InputOutputOutOfBounds.into())
        }
    }

    pub fn input_size(caller: Caller<'_, WorkerContext>) -> anyhow::Result<u32> {
        Ok(caller.data().input.len() as u32)
    }

    pub fn exit(mut caller: Caller<'_, WorkerContext>, exit_code: i32) -> anyhow::Result<()> {
        Err(TerminationReason::Exit(exit_code).into())
    }

    pub fn read_output(
        mut caller: Caller<'_, WorkerContext>,
        target_ptr: u32,
        offset: u32,
        length: u32,
    ) -> anyhow::Result<()> {
        println!("builtin::read_output()");
        let context = caller.data();
        let return_data = &context.return_data;
        if offset + length <= return_data.len() as u32 {
            let buffer = return_data[(offset as usize)..(offset as usize + length as usize)].to_vec();
            write_memory(&mut caller, target_ptr, &buffer)?;
            Ok(())
        } else {
            Err(TerminationReason::InputOutputOutOfBounds.into())
        }
    }

    pub fn output_size(caller: Caller<'_, WorkerContext>) -> anyhow::Result<u32> {
        let context = caller.data();
        Ok(context.return_data.len() as u32)
    }

    pub fn debug_log(
        mut caller: Caller<'_, WorkerContext>,
        message_ptr: u32,
        message_length: u32,
    ) -> anyhow::Result<()> {
        let message = read_memory(&mut caller, message_ptr, message_length)?;
        SyscallDebugLog::fn_impl(&message);
        Ok(())
    }

    pub fn exec(
        mut caller: Caller<'_, WorkerContext>,
        hash32_ptr: u32,
        input_ptr: u32,
        input_len: u32,
        fuel16_ptr: u32,
        state: u32,
    ) -> anyhow::Result<i32> {
        println!("builtin::exec()");
        let mut encoded_state = BytesMut::new();

        let fuel_limit = u64::MAX; // TODO(khasan) what should we put here?
        let code_hash = read_memory(&mut caller, hash32_ptr, 32)?;
        let code_hash: [u8; 32] = code_hash.as_slice().try_into().unwrap();
        let input = read_memory(&mut caller, input_ptr, input_len)?;
        let context = caller.data();
        let request = Message::ExecutionRequest {
            code_hash,
            input,
            fuel_limit,
            state,
            fuel16_ptr,
        };
        context
            .sender
            .send(request)
            .expect("failed to send execution request");
        let response = context
            .receiver
            .recv()
            .expect("failed to receive execution result");
        match response {
            Message::ExecutionResult {
                fuel_consumed,
                fuel_refunded,
                exit_code,
                output,
                fuel16_ptr,
            } => {
                let context_return_data = &mut caller.data_mut().return_data;
                context_return_data.clear();
                context_return_data.extend_from_slice(&output);
                if fuel16_ptr > 0 {
                    let mut buffer = [0u8; 16];
                    LittleEndian::write_u64(&mut buffer[..8], fuel_consumed);
                    LittleEndian::write_i64(&mut buffer[8..], fuel_refunded);
                    write_memory(&mut caller, fuel16_ptr, &buffer)?;
                }
                Ok(exit_code)
            }
            _ => panic!("unexpected message type received by the worker"),
        }
    }

    pub fn keccak256(
        mut caller: Caller<'_, WorkerContext>,
        data_ptr: u32,
        data_len: u32,
        output32_ptr: u32,
    ) -> anyhow::Result<()> {
        let data = read_memory(&mut caller, data_ptr, data_len)?;
        let hash = SyscallKeccak256::fn_impl(&data);
        write_memory(&mut caller, output32_ptr, hash.as_slice())?;
        Ok(())
    }

    pub fn charge_fuel(mut caller: Caller<'_, WorkerContext>, delta: u64) -> anyhow::Result<u64> {
        Ok(u64::MAX)
    }

    pub fn fuel(caller: Caller<'_, WorkerContext>) -> anyhow::Result<u64> {
        Ok(u64::MAX)
    }
}

fn new_linker_with_builtins(engine: &Engine) -> anyhow::Result<Linker<WorkerContext>> {
    let mut linker = Linker::new(engine);
    let module = "fluentbase_v1preview";
    linker.func_wrap(module, "_write", builtins::write)?;
    linker.func_wrap(module, "_read", builtins::read)?;
    linker.func_wrap(module, "_input_size", builtins::input_size)?;
    linker.func_wrap(module, "_exit", builtins::exit)?;
    linker.func_wrap(module, "_output_size", builtins::output_size)?;
    linker.func_wrap(module, "_read_output", builtins::read_output)?;
    linker.func_wrap(module, "_exec", builtins::exec)?;
    linker.func_wrap(module, "_debug_log", builtins::debug_log)?;
    linker.func_wrap(module, "_keccak256", builtins::keccak256)?;
    linker.func_wrap(module, "_fuel", builtins::fuel)?;
    linker.func_wrap(module, "_charge_fuel", builtins::charge_fuel)?;
    Ok(linker)
}

fn handle_one_step(executor: AsyncExecutor) -> (i32, Vec<u8>) {
    RUNTIME_STATE.with_borrow_mut(|runtime_state| {
        let next_message = executor.receive_message();
        match next_message {
            Message::ExecutionRequest {
                code_hash,
                input,
                fuel_limit,
                state,
                fuel16_ptr,
            } => {
                let call_id = runtime_state.suspend_executor(executor);
                assert!(call_id > 0);

                let params = SyscallInvocationParams {
                    code_hash: B256::from(code_hash),
                    input: Bytes::from(input),
                    fuel_limit: fuel_limit,
                    state,
                    fuel16_ptr,
                };
                let mut encoded_state = BytesMut::new();
                CompactABI::encode(&params, &mut encoded_state, 0)
                    .expect("failed to encode syscall invocation params");
                let output = encoded_state.freeze().to_vec();

                (call_id, output)
            }
            Message::WasmTerminated { reason, output } => {
                executor.worker.join().expect("worker should never panic");
                let final_exit_code = match reason {
                    TerminationReason::Exit(value) => match value {
                        0 => ExitCode::Ok.into_i32(),
                        _ => ExitCode::Panic.into_i32(),
                    }, // TODO(khasan) map wasmtime::Trap into ExitCode
                    _ => ExitCode::Err.into_i32(),
                };
                (final_exit_code, output)
            }
            _ => panic!("unexpected message type received"),
        }
    })
}
pub fn execute_wasmtime(wasm_bytecode: &[u8], input: Vec<u8>) -> (i32, Vec<u8>) {
    let executor = RUNTIME_STATE.with_borrow_mut(|runtime_state| {
        let module = runtime_state.init_module_cached(wasm_bytecode);
        AsyncExecutor::launch(module, input)
    });
    handle_one_step(executor)
}

pub fn resume_wasmtime(
    call_id: i32,
    output: Vec<u8>,
    exit_code: i32,
    fuel_consumed: u64,
    fuel_refunded: i64,
    fuel16_ptr: u32,
) -> (i32, Vec<u8>) {
    let executor = RUNTIME_STATE
        .with_borrow_mut(|runtime_state| runtime_state.take_suspended_executor(call_id));
    executor.send_message(Message::ExecutionResult {
        fuel_consumed,
        fuel_refunded,
        exit_code,
        output,
        fuel16_ptr,
    });
    handle_one_step(executor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_types::{SharedContextInput, SharedContextInputV1};

    fn insert_default_shared_context(input: &[u8]) -> Vec<u8> {
        let result = SharedContextInput::V1(SharedContextInputV1::default());
        let mut result = result.encode().unwrap().to_vec();
        result.extend_from_slice(input);
        return result;
    }

    #[test]
    fn run_identity_in_wasmtime() {
        let wasm_bytecode = include_bytes!("../../../contracts/identity/lib.wasm");
        let input = vec![1, 2, 3, 4, 5, 6];
        let (exit_code, output) =
            execute_wasmtime(wasm_bytecode, insert_default_shared_context(&input));
        assert_eq!(exit_code, 0);
        assert_eq!(input, output);
    }

    #[test]
    fn run_nitro_verifier_in_wasmtime() {
        let attestation_doc: Vec<u8> = hex::decode(include_bytes!(
            "../../../contracts/nitro/attestation-example.hex"
        ))
        .unwrap()
        .into();
        let wasm_bytecode = include_bytes!("../../../contracts/identity/lib.wasm");
        let input = attestation_doc;
        let (_, _) = execute_wasmtime(wasm_bytecode, insert_default_shared_context(&input));
        panic!("FINISHED Successfully");
    }

    #[test]
    fn wasmtime_greeting() {
        let wasm_bytecode = include_bytes!("../../../examples/greeting/lib.wasm");
        let input = Vec::new();
        let (exit_code, output) =
            execute_wasmtime(wasm_bytecode, insert_default_shared_context(&input));
        assert_eq!(exit_code, 0);
        panic!("FINISHED");
    }

    #[test]
    fn wasmtime_simple_storage() {
        let wasm_bytecode = include_bytes!("../../../examples/simple-storage/lib.wasm");
        let input = Vec::new();
        let (exit_code, output) =
            execute_wasmtime(wasm_bytecode, insert_default_shared_context(&input));
        dbg!(exit_code);
        let value = Vec::from(U256::from(2).to_le_bytes::<32>());
        let (exit_code, output) = resume_wasmtime(exit_code, value, 0, 0, 0, 0);
        assert_eq!(exit_code, 0);
        panic!("FINISHED");
    }
}
