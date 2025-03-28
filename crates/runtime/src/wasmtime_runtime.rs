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
    collections::HashMap,
    future::Future,
    mem::drop,
    pin::Pin,
    task::{Context, Poll, RawWaker, RawWakerVTable, Waker},
    time::Instant,
};
use wasmtime::{Caller, Config, Engine, Extern, Func, Linker, Memory, Module, Store, Trap};

struct AsyncExecutor {
    store: Store,
    func: Func,
    future: Pin<Box<dyn Future<Output = Result<()>>>>,

    waker: Waker,
}

impl AsyncExecutor {
    fn new(store: Store, func: Func) -> Result<Self> {
        // The code is not yet running after call_async(). Only future is created.
        // Actual execution is happeinng when Future::poll() is executed.
        let future = func.call_async(&mut store, ());
        Self {
            store: store,
            func: main,
            future: Box::pin(future),
            waker: noop_waker(),
        }
    }

    fn step(&self) {
        let context = Context::from_waker(&self.waker);
        let exit_code = match Future::poll(self.future, &mut context) {
            Poll::Ready(value) => {
                println!("Future completed with: {:?}", value);
                0
            }
            Poll::Pending => {
                unsafe {
                    (*store_ref).fuel_async_yield_interval(None)?;
                }
                call_id as i32
            }
        };
    }
}

pub struct CachingRuntime {
    recoverable_runtimes: HashMap<u32>,
    call_counter: u32,
}

impl CachingRuntime {
    pub fn new() -> Self {
        Self {
            recoverable_runtimes: HashMap::new(),
            call_counter: 0,
        }
    }
}
thread_local! {
    static CACHING_RUNTIME: RefCell<CachingRuntime> = RefCell::new(CachingRuntime::new());
}

#[derive(Debug)]
enum HostTermination {
    Exit,
    MemoryOutOfBounds,
}

impl fmt::Display for HostTermination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl StdError for HostTermination {}

mod builtins {
    use super::*;
    fn get_memory_export(caller: &mut Caller<'_, RuntimeContext>) -> Result<Memory> {
        match caller.get_export("memory") {
            Some(Extern::Memory(memory)) => Ok(memory),
            _ => Err(HostTermination::MemoryOutOfBounds.into()),
        }
    }

    fn write_memory(
        caller: &mut Caller<'_, RuntimeContext>,
        offset: u32,
        buffer: &[u8],
    ) -> Result<()> {
        let memory = get_memory_export(caller)?;
        memory
            .write(caller, offset as usize, &buffer)
            .map_err(|_| HostTermination::MemoryOutOfBounds)?;
        Ok(())
    }

    fn read_memory(
        caller: &mut Caller<'_, RuntimeContext>,
        offset: u32,
        length: u32,
    ) -> Result<Vec<u8>> {
        let memory = get_memory_export(caller)?;
        let data = memory
            .data(&caller)
            .get(offset as usize..)
            .and_then(|arr| arr.get(..length as usize));
        if data.is_none() {
            Err(HostTermination::MemoryOutOfBounds.into())
        } else {
            Ok(Vec::from(data.unwrap()))
        }
    }

    fn write(mut caller: Caller<'_, RuntimeContext>, offset: u32, length: u32) -> Result<()> {
        let data = read_memory(&mut caller, offset, length)?;
        let context = caller.data_mut();
        context.execution_result.output.extend_from_slice(&data);
        Ok(())
    }

    fn read(
        mut caller: Caller<'_, RuntimeContext>,
        target_ptr: u32,
        offset: u32,
        length: u32,
    ) -> Result<()> {
        let context = caller.data();
        if offset + length <= context.input.len() as u32 {
            let buffer =
                context.input[(offset as usize)..(offset as usize + length as usize)].to_vec();
            write_memory(&mut caller, target_ptr, &buffer)?;
            Ok(())
        } else {
            Err(HostTermination::MemoryOutOfBounds.into())
        }
    }

    fn input_size(caller: Caller<'_, RuntimeContext>) -> Result<u32> {
        Ok(caller.data().input_size())
    }

    fn exit(mut caller: Caller<'_, RuntimeContext>, exit_code: i32) -> Result<()> {
        caller.data_mut().execution_result.exit_code = exit_code;
        Err(HostTermination::Exit.into())
    }

    fn read_output(
        mut caller: Caller<'_, RuntimeContext>,
        target_ptr: u32,
        offset: u32,
        length: u32,
    ) -> Result<()> {
        println!("builtin::read_output()");
        let context = caller.data();
        let return_data = &context.execution_result.return_data;
        if offset + length <= return_data.len() as u32 {
            let buffer =
                return_data[(offset as usize)..(offset as usize + length as usize)].to_vec();
            write_memory(&mut caller, target_ptr, &buffer)?;
            Ok(())
        } else {
            Err(HostTermination::MemoryOutOfBounds.into())
        }
    }

    fn output_size(caller: Caller<'_, RuntimeContext>) -> Result<u32> {
        let context = caller.data();
        Ok(context.execution_result.return_data.len() as u32)
    }

    fn debug_log(
        mut caller: Caller<'_, RuntimeContext>,
        message_ptr: u32,
        message_length: u32,
    ) -> Result<()> {
        let message = read_memory(&mut caller, message_ptr, message_length)?;
        SyscallDebugLog::fn_impl(&message);
        Ok(())
    }

    fn exec(
        mut caller: Caller<'_, RuntimeContext>,
        hash32_ptr: u32,
        input_ptr: u32,
        input_len: u32,
        fuel16_ptr: u32,
        state: u32,
    ) -> Result<i32> {
        println!("builtin::exec()");
        let mut encoded_state = BytesMut::new();
        let fuel_limit = u64::MAX;
        let code_hash = read_memory(&mut caller, hash32_ptr, 32)?;
        let code_hash: [u8; 32] = code_hash.as_slice().try_into().unwrap();
        let input = read_memory(&mut caller, input_ptr, input_len)?;
        let params = SyscallInvocationParams {
            code_hash: B256::new(code_hash),
            input: Bytes::from(input),
            fuel_limit,
            state: state,
            fuel16_ptr: fuel16_ptr,
        };
        CompactABI::encode(&params, &mut encoded_state, 0).unwrap();

        let output = &mut caller.data_mut().execution_result.output;
        output.clear();
        assert!(output.is_empty());
        output.extend(encoded_state.freeze().to_vec());
        //Err(HostTermination::Interrupt.into())
        //Err(Trap::Interrupt.into())
        caller.fuel_async_yield_interval(Some(1))?;
        Ok(0)
    }

    fn keccak256(
        mut caller: Caller<'_, RuntimeContext>,
        data_ptr: u32,
        data_len: u32,
        output32_ptr: u32,
    ) -> Result<()> {
        let data = read_memory(&mut caller, data_ptr, data_len)?;
        let hash = SyscallKeccak256::fn_impl(&data);
        write_memory(&mut caller, output32_ptr, hash.as_slice())?;
        Ok(())
    }

    fn charge_fuel(mut caller: Caller<'_, RuntimeContext>, delta: u64) -> Result<u64> {
        Ok(u64::MAX)
    }

    fn fuel(caller: Caller<'_, RuntimeContext>) -> Result<u64> {
        Ok(u64::MAX)
    }

    pub fn linker() -> Result<Linker> {
        let mut linker = Linker::new(&engine);
        linker.func_wrap("fluentbase_v1preview", "_write", write)?;
        linker.func_wrap("fluentbase_v1preview", "_read", read)?;
        linker.func_wrap("fluentbase_v1preview", "_input_size", input_size)?;
        linker.func_wrap("fluentbase_v1preview", "_exit", exit)?;
        linker.func_wrap("fluentbase_v1preview", "_output_size", output_size)?;
        linker.func_wrap("fluentbase_v1preview", "_read_output", read_output)?;
        linker.func_wrap("fluentbase_v1preview", "_exec", exec)?;
        linker.func_wrap("fluentbase_v1preview", "_debug_log", debug_log)?;
        linker.func_wrap("fluentbase_v1preview", "_keccak256", keccak256)?;
        linker.func_wrap("fluentbase_v1preview", "_fuel", fuel)?;
        linker.func_wrap("fluentbase_v1preview", "_charge_fuel", charge_fuel)?;
        Ok(linker)
    }
}

fn resume_inner(call_id: u32, input: Vec<u8>) -> Result<(i32, Vec<u8>)> {
    Ok((0, Vec::new()))
}

fn exec_inner(wasm_bytecode: &[u8], input: Vec<u8>) -> Result<(i32, Vec<u8>)> {
    let runtime_context = RuntimeContext::root(0).with_input(input);
    let mut config = Config::new();
    config.consume_fuel(true);
    config.async_support(true);
    let engine = Engine::new(&config)?;
    let module = Module::new(&engine, wasm_bytecode)?;
    let linker = builtins::linker()?;
    let mut store = Store::new(&engine, runtime_context);
    let store_ref = &mut store as *mut Store<RuntimeContext>;
    store.set_fuel(u64::MAX);
    store.fuel_async_yield_interval(None)?;

    let instance = block_on(linker.instantiate_async(&mut store, &module))?;
    let main = instance.get_typed_func::<(), ()>(&mut store, "main")?;

    let start = Instant::now();
    let future = main.call_async(&mut store, ());
    let mut pinned_future = Box::pin(future);

    CACHING_RUNTIME.with_borrow_mut(|caching_runtime| {
        caching_runtime.call_counter += 1;
        let call_id = caching_runtime.call_counter;
        caching_runtime
            .recoverable_runtimes
            .insert(call_id, pinned_future);
        let future = caching_runtime
            .recoverable_runtimes
            .get_mut(&call_id)
            .unwrap()
            .as_mut();

        let waker = dummy_waker();
        let mut context = Context::from_waker(&waker);
        let exit_code = match Future::poll(future, &mut context) {
            Poll::Ready(value) => {
                println!("Future completed with: {:?}", value);
                0
            }
            Poll::Pending => {
                unsafe {
                    (*store_ref).fuel_async_yield_interval(None)?;
                }
                call_id as i32
            }
        };

        drop(pinned_future);

        println!("Main executed. Time elapsed is: {:?}", start.elapsed());
        Ok((exit_code, store.data().output().clone().into()))
    })
}

pub fn exec_in_wasmtime_runtime(wasm_bytecode: &[u8], input: Vec<u8>) -> (i32, Vec<u8>) {
    exec_inner(wasm_bytecode, input).unwrap()
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
            "../../../examples/nitro-verifier/attestation-example.hex"
        ))
        .unwrap()
        .into();
        let wasm_bytecode = include_bytes!("../../../examples/nitro-verifier/lib.wasm");
        let input = attestation_doc;
        let (_, _) = exec_in_wasmtime_runtime(wasm_bytecode, insert_default_shared_context(&input));
    }

    #[test]
    fn wasmtime_simple_storage() {
        let wasm_bytecode = include_bytes!("../../../examples/simple-storage/lib.wasm");
        let input = Vec::new();
        let (_, _) = exec_in_wasmtime_runtime(wasm_bytecode, insert_default_shared_context(&input));
        panic!("FINISHED");
    }
}
