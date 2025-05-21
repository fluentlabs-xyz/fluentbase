use fluentbase_build::{compile_rust_to_wasm, Config};
use fluentbase_sdk::default_compilation_config;
use std::env;

fn main() {
    if env::var("TARGET").unwrap() == "wasm32-unknown-unknown" {
        return;
    }

    let mut rwasm_config = default_compilation_config();
    rwasm_config.builtins_consume_fuel(false);
    let config = Config::default()
        .with_rerun_if_changed("src")
        .with_rerun_if_changed("Cargo.toml")
        .with_rwasm_compilation_config(Some(rwasm_config));
    compile_rust_to_wasm(config)
}
