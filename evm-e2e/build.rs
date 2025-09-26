use fluentbase_contracts::*;
use fluentbase_types::*;
use rwasm::{compile_wasmtime_module, CompilationConfig};
use std::{env, fs, io::Write, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[rustfmt::skip]
    let contracts = [
        ("modexp", FLUENTBASE_CONTRACTS_MODEXP, vec![PRECOMPILE_BIG_MODEXP]),
        ("blake2f", FLUENTBASE_CONTRACTS_BLAKE2F, vec![PRECOMPILE_BLAKE2F]),
        ("bn256", FLUENTBASE_CONTRACTS_BN256, vec![
            PRECOMPILE_BN256_ADD,
            PRECOMPILE_BN256_MUL,
            PRECOMPILE_BN256_PAIR,
        ]),
        ("evm", FLUENTBASE_CONTRACTS_EVM, vec![PRECOMPILE_EVM_RUNTIME]),
        ("identity", FLUENTBASE_CONTRACTS_IDENTITY, vec![PRECOMPILE_IDENTITY]),
        ("bls12381", FLUENTBASE_CONTRACTS_BLS12381, vec![
            PRECOMPILE_BLS12_381_G1_ADD,
            PRECOMPILE_BLS12_381_G1_MSM,
            PRECOMPILE_BLS12_381_G2_ADD,
            PRECOMPILE_BLS12_381_G2_MSM,
            PRECOMPILE_BLS12_381_PAIRING,
            PRECOMPILE_BLS12_381_MAP_G1,
            PRECOMPILE_BLS12_381_MAP_G2,
        ]),
        ("ripemd160", FLUENTBASE_CONTRACTS_RIPEMD160, vec![PRECOMPILE_RIPEMD160]),
        ("ecrecover", FLUENTBASE_CONTRACTS_ECRECOVER, vec![PRECOMPILE_SECP256K1_RECOVER]),
        ("sha256", FLUENTBASE_CONTRACTS_SHA256, vec![PRECOMPILE_SHA256]),
    ];
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let rs_path = out_dir.join("precompiled_module.rs");
    let mut f = fs::File::create(&rs_path)?;
    let mut full_list = String::new();
    for (name, contract, _addresses) in contracts {
        let wasmtime_module =
            compile_wasmtime_module(CompilationConfig::default(), contract.wasm_bytecode)
                .expect("failed to compile contract into wasmtime module");
        let raw_wasmtime_module = wasmtime_module
            .serialize()
            .expect("failed to serialize wasmtime module");
        let cwasm_name = format!("fluentbase_{}.cwasm", name);
        let cwasm_path = out_dir.join(&cwasm_name);
        fs::write(&cwasm_path, &raw_wasmtime_module)?;
        write!(
            f,
            r#"
    pub const PRECOMPILED_{}_CWASM_MODULE: &'static [u8] = include_bytes!(concat!(env!("OUT_DIR"), "/{cwasm_name}"));
            "#,
            name.to_uppercase(),
        )?;
        full_list += format!(
            "  (\"{name}\", PRECOMPILED_{}_CWASM_MODULE),\n",
            name.to_uppercase(),
        )
        .as_str();
    }
    write!(
        f,
        r#"
    pub const PRECOMPILED_MODULES: &'static [(&'static str, &'static [u8])] = &[
    {full_list}];
            "#,
    )?;
    Ok(())
}
