use fluentbase_build::{compile_go_to_wasm, is_tinygo_installed, Config};
use std::fs;

fn main() {
    if is_tinygo_installed() {
        let config = Config::default()
            .with_rerun_if_changed("main.go")
            .with_rerun_if_changed("go.mod")
            .with_rerun_if_changed("go.sum");
        compile_go_to_wasm(config)
    } else {
        let path = fs::canonicalize("fallback.wasm").unwrap();
        println!(
            "cargo:rustc-env=FLUENTBASE_WASM_ARTIFACT_PATH={}",
            path.to_str().unwrap()
        );
        println!("cargo:warning=tinygo not found, wasm may be outdated");
    }
}
