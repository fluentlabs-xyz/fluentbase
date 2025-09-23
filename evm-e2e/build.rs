use fluentbase_types::{compile_wasm_to_rwasm, keccak256};
use rwasm::{compile_wasmtime_module, CompilationConfig};
use std::{env, fs, io::Write, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let evm_runtime_bytecode = fluentbase_contracts::FLUENTBASE_CONTRACTS_EVM.wasm_bytecode;
    let wasmtime_module =
        compile_wasmtime_module(CompilationConfig::default(), evm_runtime_bytecode)
            .expect("failed to compile EVM runtime into wasmtime module");
    let raw_wasmtime_module = wasmtime_module
        .serialize()
        .expect("failed to serialize wasmtime module");
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let cwasm_name = "fluentbase_evm_runtime.cwasm";
    let cwasm_path = out_dir.join(cwasm_name);
    fs::write(&cwasm_path, &raw_wasmtime_module)?;
    let rs_path = out_dir.join("precompiled_module.rs");
    let mut f = fs::File::create(&rs_path)?;
    let rwasm_bytecode =
        compile_wasm_to_rwasm(evm_runtime_bytecode).expect("failed to compile rWasm module");
    let raw_rwasm_module = rwasm_bytecode.rwasm_module.serialize();
    let code_hash = keccak256(&raw_rwasm_module);
    println!("precompiled rwasm hash: {:?}", code_hash);
    let rwasm_name = "fluentbase_evm_runtime.rwasm";
    let rwasm_path = out_dir.join(rwasm_name);
    fs::write(&rwasm_path, &raw_rwasm_module)?;
    write!(
        f,
        r#"
pub const PRECOMPILED_RUNTIME_EVM_CWASM_MODULE: &'static [u8] = include_bytes!(concat!(env!("OUT_DIR"), "/{cwasm_name}"));
pub const PRECOMPILED_RUNTIME_EVM_RWASM_MODULE: &'static [u8] = include_bytes!(concat!(env!("OUT_DIR"), "/{rwasm_name}"));
        "#
    )?;
    Ok(())
}
