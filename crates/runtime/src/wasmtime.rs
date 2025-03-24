use wasmtime::*;
use crate::RuntimeContext;

pub fn exec_in_wasmtime_runtime(
    &self,
    wasm_bytecode: &[u8],
    input: &[u8],
) -> (i32, Vec<8>) {
    let runtime_context = RuntimeContext::root(fuel_limit)
        .with_input(input);
    let wasm_bytecode = include_bytes!("../../../../../examples/identity/lib.wasm");

    let engine = Engine::default();
    let module = Module::new(&engine, wasm_bytecode).unwrap();
    let mut store = Store::new(&engine, runtime_context);

    let mut linker = Linker::new(&engine);
    linker
        .func_wrap(
            "fluentbase_v1preview",
            "_write",
            |mut caller: Caller<'_, RuntimeContext>, offset: u32, length: u32| {
                let memory = match caller.get_export("memory") {
                    Some(Extern::Memory(memory)) => memory,
                    _ => panic!("failed to find host memory"),
                };
                let data: Vec<u8> = memory
                    .data(&caller)
                    .get(offset as usize..)
                    .and_then(|arr| arr.get(..length as usize))
                    .unwrap()
                    .into();
                let ctx = caller.data_mut();
                SyscallWrite::fn_impl(ctx, &data);
            },
        )
        .unwrap();

    linker
        .func_wrap(
            "fluentbase_v1preview",
            "_input_size",
            |caller: Caller<'_, RuntimeContext>| -> anyhow::Result<u32> {
                let size = SyscallInputSize::fn_impl(caller.data());
                println!("_input_size syscall was executed with size={}", size);
                Ok(size)
            }
        )
        .unwrap();

    linker
        .func_wrap(
            "fluentbase_v1preview",
            "_read",
            |mut caller: Caller<'_, RuntimeContext>,
             target_ptr: u32,
             offset: u32,
             length: u32| {
                // memory.write(caller.data_mut())

                let buffer = SyscallRead::fn_impl(caller.data(), offset, length).unwrap();
                let memory = match caller.get_export("memory") {
                    Some(Extern::Memory(memory)) => memory,
                    _ => panic!("failed to find host memory"),
                };
                let _ = memory.write(caller, target_ptr as usize, &buffer);
            },
        )
        .unwrap();

    linker
        .func_wrap(
            "fluentbase_v1preview",
            "_exit",
            |mut caller: Caller<'_, RuntimeContext>, exit_code: i32| -> anyhow::Result<()> {
                let exit_code = SyscallExit::fn_impl(caller.data_mut(), exit_code).unwrap_err();
                println!("_exit syscall was executed with exit code {}", exit_code);
                Err(anyhow::Error::new(exit_code))


            },
        )
        .unwrap();

    let instance = linker.instantiate(&mut store, &module).unwrap();

    let main = instance
        .get_typed_func::<(), ()>(&mut store, "main")
        .unwrap();
    match main.call(&mut store, ()) {
        Ok(_) => {},
        Err(exit_code) => {
            println!("hey here is error code {:?}", exit_code);
            println!("{:?}", store.data().output());
        },
    }

    return (0, store.data().output().clone().into());
}

