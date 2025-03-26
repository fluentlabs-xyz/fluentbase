use crate::RuntimeContext;
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
    use fluentbase_types::ExitCode;
    use wasmtime::{Caller, Extern, Memory};

    fn get_memory_export(caller: &mut Caller<'_, RuntimeContext>) -> anyhow::Result<Memory> {
        match caller.get_export("memory") {
            Some(Extern::Memory(memory)) => Ok(memory),
            _ => Err(anyhow::Error::new(ExitCode::MemoryOutOfBounds)),
        }
    }

    fn write_memory(
        caller: &mut Caller<'_, RuntimeContext>,
        offset: u32,
        buffer: &[u8],
    ) -> anyhow::Result<()> {
        let memory = get_memory_export(caller)?;
        memory
            .write(caller, offset as usize, &buffer)
            .map_err(|_| anyhow::Error::new(ExitCode::MemoryOutOfBounds))?;
        Ok(())
    }

    fn read_memory(
        caller: &mut Caller<'_, RuntimeContext>,
        offset: u32,
        length: u32,
    ) -> anyhow::Result<Vec<u8>> {
        let memory = get_memory_export(caller)?;
        let data = memory
            .data(&caller)
            .get(offset as usize..)
            .and_then(|arr| arr.get(..length as usize));
        if data.is_none() {
            Err(anyhow::Error::new(ExitCode::MemoryOutOfBounds))
        } else {
            Ok(Vec::from(data.unwrap()))
        }
    }

    pub fn write(
        mut caller: Caller<'_, RuntimeContext>,
        offset: u32,
        length: u32,
    ) -> anyhow::Result<()> {
        let data = read_memory(&mut caller, offset, length)?;
        let ctx = caller.data_mut();
        SyscallWrite::fn_impl(ctx, &data);
        Ok(())
    }

    pub fn read(
        mut caller: Caller<'_, RuntimeContext>,
        target_ptr: u32,
        offset: u32,
        length: u32,
    ) -> anyhow::Result<()> {
        let buffer = SyscallRead::fn_impl(caller.data(), offset, length)
            .map_err(|err| anyhow::Error::new(err))?;
        write_memory(&mut caller, target_ptr, &buffer)?;
        Ok(())
    }

    pub fn input_size(caller: Caller<'_, RuntimeContext>) -> anyhow::Result<u32> {
        let size = SyscallInputSize::fn_impl(caller.data());
        Ok(size)
    }

    pub fn exit(mut caller: Caller<'_, RuntimeContext>, exit_code: i32) -> anyhow::Result<()> {
        let exit_code = SyscallExit::fn_impl(caller.data_mut(), exit_code).unwrap_err();
        Err(anyhow::Error::new(exit_code))
    }

    pub fn read_output(
        mut caller: Caller<'_, RuntimeContext>,
        target_ptr: u32,
        offset: u32,
        length: u32,
    ) -> anyhow::Result<()> {
        let buffer = SyscallReadOutput::fn_impl(caller.data(), offset, length)
            .map_err(|err| anyhow::Error::new(err))?;
        write_memory(&mut caller, target_ptr, &buffer)?;
        Ok(())
    }

    pub fn output_size(caller: Caller<'_, RuntimeContext>) -> anyhow::Result<u32> {
        let context = caller.data();
        return Ok(SyscallOutputSize::fn_impl(context));
    }

    pub fn debug_log(
        mut caller: Caller<'_, RuntimeContext>,
        message_ptr: u32,
        message_length: u32,
    ) -> anyhow::Result<()> {
        let message = read_memory(&mut caller, message_ptr, message_length)?;
        SyscallDebugLog::fn_impl(&message);
        Ok(())
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
    }

    pub fn keccak256(
        mut caller: Caller<'_, RuntimeContext>,
        data_ptr: u32,
        data_len: u32,
        output32_ptr: u32,
    ) -> anyhow::Result<()> {
        let data = read_memory(&mut caller, data_ptr, data_len)?;
        let hash = SyscallKeccak256::fn_impl(&data);
        write_memory(&mut caller, output32_ptr, hash.as_slice())?;
        Ok(())
    }

    pub fn charge_fuel(mut caller: Caller<'_, RuntimeContext>, delta: u64) -> anyhow::Result<u64> {
        let context = caller.data_mut();
        return Ok(SyscallChargeFuel::fn_impl(context, delta));
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
        let (_, _) =
            exec_in_wasmtime_runtime(wasm_bytecode, insert_default_shared_context(&input));
    }
}
