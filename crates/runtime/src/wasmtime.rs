use crate::{
    instruction::{debug_log::SyscallDebugLog, keccak256::SyscallKeccak256},
    runtime::CALL_ID_COUNTER,
};
use core::{error::Error as StdError, fmt};
use ctor::ctor;
use fluentbase_codec::{bytes::BytesMut, CompactABI};
use fluentbase_genesis::GENESIS_CONTRACTS_BY_HASH;
use fluentbase_types::{
    byteorder::{ByteOrder, LittleEndian},
    get_import_linker_symbols,
    Bytes,
    ExitCode,
    SyscallInvocationParams,
    B256,
    STATE_DEPLOY,
    STATE_MAIN,
};
use std::{
    cell::RefCell,
    cmp::min,
    collections::HashMap,
    fmt::Debug,
    sync::{atomic::Ordering, mpsc},
    thread,
    thread::JoinHandle,
};
use wasmtime::{Caller, Config, Engine, Extern, Linker, Memory, Module, Store};

/// Warms up all Wasmtime modules at program startup.
#[ctor]
static MODULES_CACHE: HashMap<B256, Module> = {
    let config = Config::new();
    let engine = Engine::new(&config).unwrap();

    let mut map = HashMap::new();
    for (hash, contract) in GENESIS_CONTRACTS_BY_HASH.iter() {
        let module = unsafe {
            // Unsafe because no validations are performed on the module bytes.
            // So only trusted modules should be used.
            Module::deserialize(&engine, &contract.wasmtime_module_bytes).unwrap()
        };
        map.insert(hash.clone(), module);
    }
    map
};

struct WorkerContext {
    sender: mpsc::Sender<Message>,
    receiver: mpsc::Receiver<Message>,
    input: Vec<u8>,
    output: Vec<u8>,
    return_data: Vec<u8>,
    fuel_limit: u64,
    fuel_consumed: u64,
    fuel_refunded: i64,
    state: u32,
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
        fuel_consumed: u64,
        fuel_refunded: i64,
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
        fuel_consumed: u64,
        fuel_refunded: i64,
        reason: TerminationReason,
        output: Vec<u8>,
    },
}
struct AsyncExecutor {
    sender: mpsc::Sender<Message>,
    receiver: mpsc::Receiver<Message>,
    worker: JoinHandle<()>,
    fuel_consumed_checkpoint: u64,
    fuel_refunded_checkpoint: i64,
}

impl AsyncExecutor {
    pub fn calc_fuel_since_checkpoint(
        &mut self,
        fuel_consumed: u64,
        fuel_refunded: i64,
    ) -> (u64, i64) {
        let result = (
            fuel_consumed - self.fuel_consumed_checkpoint,
            fuel_refunded - self.fuel_refunded_checkpoint,
        );
        self.fuel_consumed_checkpoint = fuel_consumed;
        self.fuel_refunded_checkpoint = fuel_refunded;
        result
    }
    // Module is expected to be preverified beforehand.
    // Therefore, this function should never encounter errors during wasm instance initialization.
    pub fn launch(module: &Module, input: Vec<u8>, fuel_limit: u64, state: u32) -> Self {
        let (executor_sender, worker_receiver) = mpsc::channel();
        let (worker_sender, executor_receiver) = mpsc::channel();
        let worker_context = WorkerContext {
            sender: worker_sender,
            receiver: worker_receiver,
            input,
            output: Vec::new(),
            return_data: Vec::new(),
            fuel_limit,
            fuel_consumed: 0,
            fuel_refunded: 0,
            state,
        };
        let engine = module.engine();
        let linker = new_linker_with_builtins(engine).unwrap();
        let mut store = Store::new(engine, worker_context);

        verify_linker(&linker, &mut store);

        let instance = linker
            .instantiate(&mut store, &module)
            .expect("bytecode should be preverified");
        let func = match state {
            STATE_MAIN => instance.get_typed_func::<(), ()>(&mut store, "main"),
            STATE_DEPLOY => instance.get_typed_func::<(), ()>(&mut store, "deploy"),
            _ => panic!("unknown state, only main and deploy states are supported"),
        };
        let func = func.expect("wasm bytecode should have main and deploy exports");

        let worker = thread::spawn(move || {
            let reason = match func.call(&mut store, ()) {
                Ok(()) => TerminationReason::Exit(0),
                Err(e) => {
                    if let Some(reason) = e.downcast_ref::<TerminationReason>() {
                        reason.clone()
                    } else if let Some(trap) = e.downcast_ref::<wasmtime::Trap>() {
                        TerminationReason::Trap(trap.clone())
                    } else {
                        panic!("unexpected error occurred during wasm guest execution");
                    }
                }
            };
            let message = Message::WasmTerminated {
                reason,
                output: store.data().output.clone(),
                fuel_consumed: store.data().fuel_consumed,
                fuel_refunded: store.data().fuel_refunded,
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
            fuel_consumed_checkpoint: 0,
            fuel_refunded_checkpoint: 0,
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
    suspended_executors: HashMap<i32, AsyncExecutor>,
}

impl RuntimeState {
    fn new() -> Self {
        Self {
            suspended_executors: HashMap::new(),
        }
    }

    fn suspend_executor(&mut self, executor: AsyncExecutor) -> i32 {
        let call_id = CALL_ID_COUNTER.fetch_add(1, Ordering::SeqCst) as i32;
        let existing = self.suspended_executors.insert(call_id, executor);
        assert!(existing.is_none());
        call_id
    }

    fn take_suspended_executor(&mut self, call_id: i32) -> Option<AsyncExecutor> {
        self.suspended_executors.remove(&call_id)
    }
}

mod builtins {
    use super::*;
    use crate::instruction::secp256k1_recover::SyscallSecp256k1Recover;

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

    pub fn exit(_caller: Caller<'_, WorkerContext>, exit_code: i32) -> anyhow::Result<()> {
        Err(TerminationReason::Exit(exit_code).into())
    }

    pub fn read_output(
        mut caller: Caller<'_, WorkerContext>,
        target_ptr: u32,
        offset: u32,
        length: u32,
    ) -> anyhow::Result<()> {
        let context = caller.data();
        let return_data = &context.return_data;
        if offset + length <= return_data.len() as u32 {
            let buffer =
                return_data[(offset as usize)..(offset as usize + length as usize)].to_vec();
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
        let remaining_fuel = caller.data().fuel_limit - caller.data().fuel_consumed;
        let fuel_limit = if fuel16_ptr > 0 {
            let fuel_buffer = read_memory(&mut caller, fuel16_ptr, 16)?;
            let fuel_limit = LittleEndian::read_i64(&fuel_buffer[..8]) as u64;
            let _fuel_refund = LittleEndian::read_i64(&fuel_buffer[8..]);
            if fuel_limit > 0 {
                min(fuel_limit, remaining_fuel)
            } else {
                0
            }
        } else {
            remaining_fuel
        };
        let code_hash = read_memory(&mut caller, hash32_ptr, 32)?;
        let code_hash: [u8; 32] = code_hash
            .as_slice()
            .try_into()
            .expect("code hash should be 32 bytes");
        let input = read_memory(&mut caller, input_ptr, input_len)?;

        let context = caller.data();
        let request = Message::ExecutionRequest {
            fuel_consumed: context.fuel_consumed,
            fuel_refunded: context.fuel_refunded,
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

    pub fn charge_fuel_manually(
        mut caller: Caller<'_, WorkerContext>,
        fuel_consumed: u64,
        fuel_refunded: i64,
    ) -> anyhow::Result<u64> {
        let context = caller.data_mut();
        let new_fuel_consumed = context
            .fuel_consumed
            .checked_add(fuel_consumed)
            .unwrap_or(u64::MAX);
        if new_fuel_consumed > context.fuel_limit {
            return Err(wasmtime::Trap::OutOfFuel.into());
        }
        context.fuel_consumed = new_fuel_consumed;
        context.fuel_refunded = context
            .fuel_refunded
            .checked_add(fuel_refunded)
            .unwrap_or(i64::MAX);
        Ok(context.fuel_limit - context.fuel_consumed)
    }

    pub fn charge_fuel(
        _caller: Caller<'_, WorkerContext>,
        _fuel_consumed: u64,
    ) -> anyhow::Result<()> {
        // all contracts running in wasmtime runtime expect to
        // charge fuel manually through `charge_fuel_manually` builtin
        Err(wasmtime::Trap::UnreachableCodeReached.into())
    }

    pub fn fuel(caller: Caller<'_, WorkerContext>) -> anyhow::Result<u64> {
        let context = caller.data();
        Ok(context.fuel_limit - context.fuel_consumed)
    }

    pub fn secp256k1_recover(
        mut caller: Caller<'_, WorkerContext>,
        digest32_offset: u32,
        sig64_offset: u32,
        output65_offset: u32,
        rec_id: u32,
    ) -> anyhow::Result<i32> {
        let digest = read_memory(&mut caller, digest32_offset, 32)?;
        let digest: [u8; 32] = digest
            .as_slice()
            .try_into()
            .expect("digest should be 32 bytes");
        let digest = B256::from(digest);
        let sig = read_memory(&mut caller, sig64_offset, 64)?;
        let sig: [u8; 64] = sig
            .as_slice()
            .try_into()
            .expect("signature should be 64 bytes");
        let hash = SyscallSecp256k1Recover::fn_impl(&digest, &sig, rec_id as u8);
        if let Some(hash) = hash {
            let hash = hash.as_slice();
            write_memory(&mut caller, output65_offset, &hash)?;
            Ok(0)
        } else {
            Ok(1)
        }
    }

    pub fn forward_output(
        mut caller: Caller<'_, WorkerContext>,
        offset: u32,
        length: u32,
    ) -> anyhow::Result<()> {
        let context = caller.data_mut();
        let return_data = &context.return_data;
        if offset + length <= return_data.len() as u32 {
            let ret_data = &return_data[(offset as usize)..(offset as usize + length as usize)];
            context.output.extend_from_slice(ret_data);
            Ok(())
        } else {
            Err(wasmtime::Trap::MemoryOutOfBounds.into())
        }
    }

    pub fn state(caller: Caller<'_, WorkerContext>) -> anyhow::Result<u32> {
        Ok(caller.data().state)
    }

    pub fn preimage_copy(
        _caller: Caller<'_, WorkerContext>,
        _hash32_ptr: u32,
        _preimage_ptr: u32,
    ) -> anyhow::Result<()> {
        Err(wasmtime::Trap::UnreachableCodeReached.into())
    }

    pub fn preimage_size(
        _caller: Caller<'_, WorkerContext>,
        _hash32_ptr: u32,
    ) -> anyhow::Result<u32> {
        Err(wasmtime::Trap::UnreachableCodeReached.into())
    }

    pub fn resume(
        _caller: Caller<'_, WorkerContext>,
        _call_id: u32,
        _return_data_ptr: u32,
        _return_data_len: u32,
        _exit_code: i32,
        _fuel16_ptr: u32,
    ) -> anyhow::Result<i32> {
        // should never be called in this context
        Err(wasmtime::Trap::UnreachableCodeReached.into())
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
    linker.func_wrap(module, "_secp256k1_recover", builtins::secp256k1_recover)?;
    linker.func_wrap(module, "_forward_output", builtins::forward_output)?;
    linker.func_wrap(module, "_preimage_copy", builtins::preimage_copy)?;
    linker.func_wrap(module, "_preimage_size", builtins::preimage_size)?;
    linker.func_wrap(module, "_state", builtins::state)?;
    linker.func_wrap(module, "_resume", builtins::resume)?;
    linker.func_wrap(
        module,
        "_charge_fuel_manually",
        builtins::charge_fuel_manually,
    )?;
    Ok(linker)
}

fn handle_one_step(mut executor: AsyncExecutor) -> (u64, i64, i32, Vec<u8>) {
    let next_message = executor.receive_message();

    match next_message {
        Message::ExecutionRequest {
            fuel_consumed,
            fuel_refunded,
            code_hash,
            input,
            fuel_limit,
            state,
            fuel16_ptr,
        } => {
            let (fuel_consumed_delta, fuel_refunded_delta) =
                executor.calc_fuel_since_checkpoint(fuel_consumed, fuel_refunded);
            let call_id = RUNTIME_STATE
                .with_borrow_mut(|runtime_state| runtime_state.suspend_executor(executor));
            assert!(call_id > 0);

            let params = SyscallInvocationParams {
                code_hash: B256::from(code_hash),
                input: Bytes::from(input),
                fuel_limit,
                state,
                fuel16_ptr,
            };
            let mut encoded_state = BytesMut::new();
            CompactABI::encode(&params, &mut encoded_state, 0)
                .expect("failed to encode syscall invocation params");
            let output = encoded_state.freeze().to_vec();

            (fuel_consumed_delta, fuel_refunded_delta, call_id, output)
        }
        Message::WasmTerminated {
            fuel_consumed,
            fuel_refunded,
            reason,
            output,
        } => {
            let (fuel_consumed_delta, fuel_refunded_tick) =
                executor.calc_fuel_since_checkpoint(fuel_consumed, fuel_refunded);
            executor.worker.join().expect("worker should never panic");
            let final_exit_code = match reason {
                TerminationReason::Exit(value) => value,
                TerminationReason::Trap(trap) => match trap {
                    wasmtime::Trap::UnreachableCodeReached => {
                        ExitCode::UnreachableCodeReached.into_i32()
                    }
                    wasmtime::Trap::MemoryOutOfBounds => ExitCode::MemoryOutOfBounds.into_i32(),
                    wasmtime::Trap::TableOutOfBounds => ExitCode::TableOutOfBounds.into_i32(),
                    wasmtime::Trap::IndirectCallToNull => ExitCode::IndirectCallToNull.into_i32(),
                    wasmtime::Trap::IntegerDivisionByZero => {
                        ExitCode::IntegerDivisionByZero.into_i32()
                    }
                    wasmtime::Trap::IntegerOverflow => ExitCode::IntegerOverflow.into_i32(),
                    wasmtime::Trap::BadConversionToInteger => {
                        ExitCode::BadConversionToInteger.into_i32()
                    }
                    wasmtime::Trap::StackOverflow => ExitCode::StackOverflow.into_i32(),
                    wasmtime::Trap::BadSignature => ExitCode::BadSignature.into_i32(),
                    wasmtime::Trap::OutOfFuel => ExitCode::OutOfFuel.into_i32(),
                    wasmtime::Trap::HeapMisaligned => ExitCode::UnknownError.into_i32(),
                    wasmtime::Trap::Interrupt => ExitCode::UnknownError.into_i32(),
                    wasmtime::Trap::AlwaysTrapAdapter => ExitCode::UnknownError.into_i32(),
                    wasmtime::Trap::AtomicWaitNonSharedMemory => ExitCode::UnknownError.into_i32(),
                    wasmtime::Trap::NullReference => ExitCode::UnknownError.into_i32(),
                    wasmtime::Trap::ArrayOutOfBounds => ExitCode::UnknownError.into_i32(),
                    wasmtime::Trap::AllocationTooLarge => ExitCode::UnknownError.into_i32(),
                    wasmtime::Trap::CastFailure => ExitCode::UnknownError.into_i32(),
                    wasmtime::Trap::CannotEnterComponent => ExitCode::UnknownError.into_i32(),
                    wasmtime::Trap::NoAsyncResult => ExitCode::UnknownError.into_i32(),
                    _ => ExitCode::UnknownError.into_i32(),
                },
                _ => ExitCode::Err.into_i32(),
            };
            (
                fuel_consumed_delta,
                fuel_refunded_tick,
                final_exit_code,
                output,
            )
        }
        _ => panic!("unexpected message type received"),
    }
}

pub fn execute(
    bytecode_hash: &B256,
    input: Vec<u8>,
    fuel_limit: u64,
    state: u32,
) -> (u64, i64, i32, Vec<u8>) {
    let module = MODULES_CACHE
        .get(bytecode_hash)
        .expect("module should be cached");
    let executor = AsyncExecutor::launch(module, input, fuel_limit, state);
    handle_one_step(executor)
}

pub fn try_resume(
    call_id: i32,
    output: Vec<u8>,
    exit_code: i32,
    fuel_consumed: u64,
    fuel_refunded: i64,
    fuel16_ptr: u32,
) -> Option<(u64, i64, i32, Vec<u8>)> {
    let executor = RUNTIME_STATE
        .with_borrow_mut(|runtime_state| runtime_state.take_suspended_executor(call_id))?;
    executor.send_message(Message::ExecutionResult {
        fuel_consumed,
        fuel_refunded,
        exit_code,
        output,
        fuel16_ptr,
    });
    Some(handle_one_step(executor))
}

fn verify_linker(linker: &Linker<WorkerContext>, store: &mut Store<WorkerContext>) {
    let mut imported_symbols: Vec<&str> = linker.iter(store).map(|(_, name, _)| name).collect();
    imported_symbols.sort();
    let expected_symbols = get_import_linker_symbols();
    // TODO(khasan) verify signature of each function, not just the name
    assert_eq!(
        imported_symbols, expected_symbols,
        "imported symbols mismatch"
    );
}
