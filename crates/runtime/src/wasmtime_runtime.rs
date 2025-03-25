use crate::RuntimeContext;
use fluentbase_types::{SharedContextInput, SharedContextInputV1};
use wasmtime::{Engine, Linker, Module, Store};

const MODULE: &str = "fluentbase_v1preview";

mod builtins {
    use crate::{
        instruction::{
            charge_fuel::SyscallChargeFuel,
            debug_log::SyscallDebugLog,
            exec::SyscallExec,
            exit::SyscallExit,
            fuel::SyscallFuel,
            input_size::SyscallInputSize,
            keccak256::SyscallKeccak256,
            output_size::SyscallOutputSize,
            read::SyscallRead,
            read_output::SyscallReadOutput,
            write::SyscallWrite,
        },
        RuntimeContext,
    };
    use wasmtime::{Caller, Extern, Memory};

    fn get_memory_export(caller: &mut Caller<'_, RuntimeContext>) -> Memory {
        let memory = match caller.get_export("memory") {
            Some(Extern::Memory(memory)) => memory,
            _ => panic!("failed to find host memory"), // TODO(khasan) Get rid of panic here
        };
        return memory;
    }

    fn write_memory(
        caller: &mut Caller<'_, RuntimeContext>,
        offset: u32,
        buffer: &[u8],
    ) -> anyhow::Result<()> {
        let memory = get_memory_export(caller);
        return memory.write(caller, offset as usize, &buffer);
    }

    fn read_memory( // TODO(khasan) use read/write memory functions in syscalls
        caller: &mut Caller<'_, RuntimeContext>,
        offset: u32,
        length: u32,
    ) -> anyhow::Result<Vec<u8>> {
        let memory = get_memory_export(&mut caller);
        return memory
            .data(&caller)
            .get(offset as usize..)
            .and_then(|arr| arr.get(..length as usize))
            .into();
    }

    pub fn write(mut caller: Caller<'_, RuntimeContext>, offset: u32, length: u32) {
        let ctx = caller.data_mut();
        SyscallWrite::fn_impl(ctx, &data);
    }

    pub fn read(mut caller: Caller<'_, RuntimeContext>, target_ptr: u32, offset: u32, length: u32) {
        let buffer = SyscallRead::fn_impl(caller.data(), offset, length).unwrap();
        let memory = get_memory_export(&mut caller);
        // TODO(khasan) Handle error returned from
    }

    pub fn input_size(caller: Caller<'_, RuntimeContext>) -> anyhow::Result<u32> {
        let size = SyscallInputSize::fn_impl(caller.data());
        Ok(size)
    }

    pub fn exit(mut caller: Caller<'_, RuntimeContext>, exit_code: i32) -> anyhow::Result<()> {
        let exit_code = SyscallExit::fn_impl(caller.data_mut(), exit_code).unwrap_err();
        println!("_exit syscall was executed with exit code {}", exit_code);
        Err(anyhow::Error::new(exit_code))
    }

    pub fn read_output(
        mut caller: Caller<'_, RuntimeContext>,
        target_ptr: u32,
        offset: u32,
        length: u32,
    ) {
        let buffer = SyscallReadOutput::fn_impl(caller.data(), offset, length).unwrap();
        let memory = get_memory_export(&mut caller);
        let _ = memory.write(caller, target_ptr as usize, &buffer);
    }

    pub fn output_size(caller: Caller<'_, RuntimeContext>) -> anyhow::Result<u32> {
        let context = caller.data();
        return Ok(SyscallOutputSize::fn_impl(context));
    }

    pub fn debug_log(caller: Caller<'_, RuntimeContext>, message_ptr: u32, message_length: u32) {
        todo!();
    }

    pub fn exec(
        caller: Caller<'_, RuntimeContext>,
        hash32_ptr: u32,
        input_ptr: u32,
        input_len: u32,
        fuel16_ptr: u32,
        state: u32,
    ) -> anyhow::Result<i32> {
        todo!();
        Ok(0)
    }

    pub fn keccak256(
        caller: Caller<'_, RuntimeContext>,
        data_ptr: u32,
        data_len: u32,
        output32_ptr: u32,
    ) -> anyhow::Result<()> {
        todo!();
        Ok(())
    }

    pub fn charge_fuel(caller: Caller<'_, RuntimeContext>, _delta: u64) -> anyhow::Result<u64> {
        let context = caller.data();
        return Ok(SyscallFuel::fn_impl(context));
    }

    pub fn fuel(caller: Caller<'_, RuntimeContext>) -> anyhow::Result<u64> {
        let context = caller.data();
        return Ok(SyscallFuel::fn_impl(context));
    }
}

fn exec_internal(wasm_bytecode: &[u8], input: Vec<u8>) -> anyhow::Result<(i32, Vec<u8>)> {
    let runtime_context = RuntimeContext::root(0).with_input(input);
    let engine = Engine::default();
    let module = Module::new(&engine, wasm_bytecode)?;
    let mut store = Store::new(&engine, runtime_context);

    let mut linker = Linker::new(&engine);
    linker.func_wrap(MODULE, "_write", builtins::write)?;
    linker.func_wrap(MODULE, "_read", builtins::read)?;
    linker.func_wrap(MODULE, "_input_size", builtins::input_size)?;
    linker.func_wrap(MODULE, "_exit", builtins::exit)?;
    linker.func_wrap(MODULE, "_output_size", builtins::output_size)?;
    linker.func_wrap(MODULE, "_read_output", builtins::read_output)?;
    linker.func_wrap(MODULE, "_exec", builtins::exec)?;
    linker.func_wrap(MODULE, "_debug_log", builtins::debug_log)?;
    linker.func_wrap(MODULE, "_keccak256", builtins::keccak256)?;
    linker.func_wrap(MODULE, "_fuel", builtins::fuel)?;
    linker.func_wrap(MODULE, "_charge_fuel", builtins::charge_fuel)?;

    let instance = linker.instantiate(&mut store, &module)?;
    let main = instance.get_typed_func::<(), ()>(&mut store, "main")?;
    match main.call(&mut store, ()) {
        Ok(_) => {}
        Err(exit_code) => {
            println!("hey here is error code {:?}", exit_code);
            println!("{:?}", store.data().output());
        }
    }
    return Ok((0, store.data().output().clone().into()));
}

pub fn exec_in_wasmtime_runtime(wasm_bytecode: &[u8], input: Vec<u8>) -> (i32, Vec<u8>) {
    exec_internal(wasm_bytecode, input).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert_default_shared_context(input: &[u8]) -> Vec<u8> {
        let result = SharedContextInput::V1(SharedContextInputV1::default());
        let mut result = result.encode().unwrap().to_vec();
        result.extend_from_slice(input);
        return result;
    }

    #[test]
    fn wasmtime_identity() {
        let wasm_bytecode = include_bytes!("../../../examples/identity/lib.wasm");
        let input = vec![1, 2, 3, 4, 5, 6];
        let (exit_code, output) =
            exec_in_wasmtime_runtime(wasm_bytecode, insert_default_shared_context(&input));
        assert_eq!(input, output);
    }
}
