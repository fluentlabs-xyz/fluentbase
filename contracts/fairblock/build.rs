use fluentbase_build::{
    copy_wasm_and_wat,
    generate_build_output_file,
    go_to_wasm,
    is_tinygo_installed,
    wasm_to_rwasm
    ,
};
use fluentbase_sdk::default_compilation_config;
use std::{env, fs};

fn main() {
    if env::var("TARGET").unwrap() == "wasm32-unknown-unknown" {
        return;
    }

    println!("cargo:rerun-if-changed=lib.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=main.go");
    println!("cargo:rerun-if-changed=go.mod");
    println!("cargo:rerun-if-changed=go.sum");
    println!("cargo:rerun-if-changed=fallback.wasm");

    let wasm_path = if is_tinygo_installed() {
        go_to_wasm()
    } else {
        fs::canonicalize("fallback.wasm").unwrap()
    };

    copy_wasm_and_wat(&wasm_path);
    let rwasm_config = default_compilation_config().with_builtins_consume_fuel(false);
    let rwasm_path = wasm_to_rwasm(&wasm_path, rwasm_config);

    println!(
        "cargo:rustc-env=FLUENTBASE_WASM_ARTIFACT_PATH={}",
        wasm_path.to_str().unwrap()
    );

    generate_build_output_file(&wasm_path, &rwasm_path);
}
