use crate::{
    instruction::{debug_log::SyscallDebugLog, keccak256::SyscallKeccak256},
    RuntimeContext,
};
use anyhow::Result;
use core::{error::Error as StdError, fmt};
use fluentbase_codec::{bytes::BytesMut, CompactABI};
use fluentbase_types::{Bytes, FixedBytes, SyscallInvocationParams, B256};
use futures::{executor::block_on, task::noop_waker};
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
}

#[derive(Debug, Clone)]
enum TerminationReason {
    Exit(i32), // TODO: rename to VolonteerExit
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
        fuel_limit: u32,
        state: u32,
    },
    ExecutionResponse {
        fuel_consumed: u64,
        fuel_refunded: i64,
        exit_code: i32,
        output: Vec<u8>,
    },
    WasmTerminated {
        reason: TerminationReason,
        output: Vec<u8>,
    },
}

struct AsyncExecutor {
    sender: mpsc::Sender<Message>,
    receiver: mpsc::Receiver<Message>,
    worker_join_handle: JoinHandle<()>,
}

impl AsyncExecutor {
    pub fn launch(module: &Module, input: Vec<u8>) -> anyhow::Result<Self> {
        let (executor_sender, worker_receiver) = mpsc::channel();
        let (worker_sender, executor_receiver) = mpsc::channel();
        let worker_context = WorkerContext {
            sender: worker_sender,
            receiver: worker_receiver,
            input: input,
            output: Vec::new(),
        };
        let engine = module.engine();
        let linker = new_linker_with_builtins(engine)?;
        let mut store = Store::new(engine, worker_context);

        let instance = linker.instantiate(&mut store, &module)?;
        let main = instance.get_typed_func::<(), ()>(&mut store, "main")?;

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

        Ok(Self {
            sender: executor_sender,
            receiver: executor_receiver,
            worker_join_handle: worker,
        })
    }
    pub fn receive_message(&self) -> anyhow::Result<Message> {
        let message = self.receiver.recv()?;
        Ok(message)
    }
    pub fn send_message(&self, message: Message) -> anyhow::Result<()> {
        let _ = self.sender.send(message)?;
        Ok(())
    }
}

thread_local! {
    static RUNTIME_STATE: RefCell<RuntimeState> = RefCell::new(RuntimeState::new());
}
struct RuntimeState {
    modules_cache: HashMap<u64, Module>,
    suspended_executors: HashMap<u64, AsyncExecutor>,
}
impl RuntimeState {
    fn new() -> Self {
        // let config = Config::new();
        // let engine = Engine::new(&config).unwrap();
        // let linker = new_linker_with_builtins(&engine).unwrap();
        Self {
            modules_cache: HashMap::new(),
            suspended_executors: HashMap::new(),
        }
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
        let output = &context.output;
        if offset + length <= output.len() as u32 {
            let buffer = output[(offset as usize)..(offset as usize + length as usize)].to_vec();
            write_memory(&mut caller, target_ptr, &buffer)?;
            Ok(())
        } else {
            Err(TerminationReason::InputOutputOutOfBounds.into())
        }
    }

    pub fn output_size(caller: Caller<'_, WorkerContext>) -> anyhow::Result<u32> {
        let context = caller.data();
        Ok(context.output.len() as u32)
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
        let fuel_limit = u32::MAX;
        let code_hash = read_memory(&mut caller, hash32_ptr, 32)?;
        let code_hash: [u8; 32] = code_hash.as_slice().try_into().unwrap();
        let input = read_memory(&mut caller, input_ptr, input_len)?;
        let context = caller.data();
        let request = Message::ExecutionRequest {
            code_hash,
            input,
            fuel_limit,
            state,
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
            Message::ExecutionResponse {
                fuel_consumed,
                fuel_refunded,
                exit_code,
                output,
            } => {
                let context_output = &mut caller.data_mut().output;
                context_output.clear();
                context_output.extend_from_slice(&output);
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
fn resume_inner(call_id: u32, input: Vec<u8>) -> anyhow::Result<(i32, Vec<u8>)> {
    Ok((0, Vec::new()))
}

fn exec_inner(wasm_bytecode: &[u8], input: Vec<u8>) -> anyhow::Result<(i32, Vec<u8>)> {
    Ok((0, vec![]))

    // let message = main_receiver.recv();
    // match message {
    //     Ok(message) => match message {
    //         Message::ExecutionRequest {
    //             code_hash,
    //             input,
    //             fuel_limit,
    //             state,
    //         } => {
    //             println!("Executing contract:");
    //             println!("Code Hash: {:?}", code_hash);
    //             println!("Input: {:?}", input);
    //             println!("Fuel Limit: {}", fuel_limit);
    //             println!("State: {}", state);
    //             Ok((0, vec![]))
    //         }
    //         Message::WasmTerminated { reason, output } => {
    //             println!("WASM Terminated:");
    //             println!("Reason: {:?}", reason);
    //             println!("Output: {:?}", output);
    //             worker
    //                 .join()
    //                 .expect("worker should exit successfully after termination message");
    //             Ok((0, vec![]))
    //         }
    //         _ => panic!("unexpected message type received from worker"),
    //     },
    //     Err(err) => {
    //         let worker_result = worker.join();
    //         match worker_result {
    //             Ok(()) => panic!("worker finished execution without sending termination reason"),
    //             Err(_) => Err(anyhow::Error::msg("worker panicked")),
    //         }
    //     }
    // }
}

pub fn exec_in_wasmtime_runtime(wasm_bytecode: &[u8], input: Vec<u8>) -> (i32, Vec<u8>) {
    let result: anyhow::Result<AsyncExecutor> = RUNTIME_STATE.with_borrow_mut(|runtime_state| {
        let mut hasher = DefaultHasher::new();
        wasm_bytecode.hash(&mut hasher);
        let hash = hasher.finish();
        let module: &Module = match runtime_state.modules_cache.get(&hash) {
            Some(value) => value,
            None => {
                let mut config = Config::new();
                let engine = Engine::new(&config)?;
                let module = Module::new(&engine, wasm_bytecode)?;
                // I'll remove this 3 lookups once I get more experience in rust (never)
                let _ = runtime_state.modules_cache.insert(hash.clone(), module);
                runtime_state.modules_cache.get(&hash).unwrap()
            }
        };
        let executor = AsyncExecutor::launch(module, input)?;
        Ok(executor)
    });
    (0, vec![])
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
        let wasm_bytecode = include_bytes!("../../../examples/identity/lib.wasm");
        let input = vec![1, 2, 3, 4, 5, 6];
        let (_, output) =
            exec_in_wasmtime_runtime(wasm_bytecode, insert_default_shared_context(&input));
        assert_eq!(input, output);
    }

    #[test]
    fn run_nitro_verifier_in_wasmtime() {
        let attestation_doc: Vec<u8> = hex::decode(include_bytes!(
            "../../../contracts/nitro/attestation-example.hex"
        ))
        .unwrap()
        .into();
        let wasm_bytecode = include_bytes!("../../../examples/nitro-verifier/lib.wasm");
        let input = attestation_doc;
        let (_, _) = exec_in_wasmtime_runtime(wasm_bytecode, insert_default_shared_context(&input));
        panic!("FINISHED Successfully");
    }

    #[test]
    fn wasmtime_simple_storage() {
        let wasm_bytecode = include_bytes!("../../../examples/simple-storage/lib.wasm");
        let input = Vec::new();
        let (_, _) = exec_in_wasmtime_runtime(wasm_bytecode, insert_default_shared_context(&input));
        panic!("FINISHED");
    }
}
