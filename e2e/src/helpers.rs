use core::str::from_utf8;
use fluentbase_codec::Encoder;
use fluentbase_runtime::{ExecutionResult, Runtime, RuntimeContext};
use fluentbase_types::{
    create_import_linker,
    ExitCode,
    SharedContextInputV1,
    SysFuncIdx::STATE,
    STATE_DEPLOY,
    STATE_MAIN,
};
use rwasm::{
    engine::{bytecode::Instruction, RwasmConfig, StateRouterConfig},
    rwasm::{instruction::InstructionExtra, BinaryFormat, BinaryFormatWriter, RwasmModule},
    Error,
};

pub(crate) fn run_with_default_context(wasm_binary: Vec<u8>, input_data: &[u8]) -> (Vec<u8>, i32) {
    let rwasm_binary = wasm2rwasm(wasm_binary.as_slice()).unwrap();
    let mut context_input = SharedContextInputV1 {
        block: Default::default(),
        tx: Default::default(),
        contract: Default::default(),
    }
    .encode_to_vec(0);
    context_input.extend_from_slice(input_data);
    let ctx = RuntimeContext::new(rwasm_binary)
        .with_state(STATE_MAIN)
        .with_fuel_limit(100_000_000_000)
        .with_input(context_input)
        .with_tracer();
    let mut runtime = Runtime::new(ctx);
    runtime.data_mut().clear_output();
    let result = runtime.call();
    println!(
        "exit_code: {} ({})",
        result.exit_code,
        ExitCode::from(result.exit_code)
    );
    println!(
        "output: 0x{} ({})",
        hex::encode(&result.output),
        from_utf8(&result.output).unwrap_or("can't decode utf-8")
    );
    println!("fuel consumed: {}", result.fuel_consumed);
    if result.exit_code != 0 {
        let logs = &runtime.store().tracer().unwrap().logs;
        println!("execution trace ({} steps):", logs.len());
        for log in logs.iter().rev().take(100).rev() {
            if let Some(value) = log.opcode.aux_value() {
                println!(
                    " - pc={} opcode={}({})",
                    log.program_counter, log.opcode, value
                );
            } else {
                println!(" - pc={} opcode={}", log.program_counter, log.opcode);
            }
        }
    } else {
        println!(
            "trace steps: {}",
            runtime.store().tracer().unwrap().logs.len()
        );
    }
    (result.output.into(), result.exit_code)
}

#[allow(dead_code)]
pub(crate) fn catch_panic(ctx: &ExecutionResult) {
    if ctx.exit_code != -71 {
        return;
    }
    println!(
        "panic with err: {}",
        std::str::from_utf8(&ctx.output).unwrap()
    );
}

#[inline(always)]
pub fn rwasm_module(wasm_binary: &[u8]) -> Result<RwasmModule, Error> {
    let mut config = RwasmModule::default_config(None);
    config.rwasm_config(RwasmConfig {
        state_router: Some(StateRouterConfig {
            states: Box::new([
                ("deploy".to_string(), STATE_DEPLOY),
                ("main".to_string(), STATE_MAIN),
            ]),
            opcode: Instruction::Call(STATE.into()),
        }),
        entrypoint_name: None,
        import_linker: Some(create_import_linker()),
        wrap_import_functions: true,
    });
    RwasmModule::compile_with_config(wasm_binary, &config)
}

#[inline(always)]
pub fn wasm2rwasm(wasm_binary: &[u8]) -> Result<Vec<u8>, ExitCode> {
    let rwasm_module = rwasm_module(wasm_binary);
    if rwasm_module.is_err() {
        return Err(ExitCode::CompilationError);
    }
    let rwasm_module = rwasm_module.unwrap();
    let length = rwasm_module.encoded_length();
    let mut rwasm_bytecode = vec![0u8; length];
    let mut binary_format_writer = BinaryFormatWriter::new(&mut rwasm_bytecode);
    rwasm_module
        .write_binary(&mut binary_format_writer)
        .expect("failed to encode rwasm bytecode");
    Ok(rwasm_bytecode)
}
