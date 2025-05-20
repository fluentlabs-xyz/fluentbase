use fluentbase_build::{compile_go_to_wasm, is_tinygo_installed, Config};

fn main() {
    if is_tinygo_installed() {
        let config = Config::default()
            .with_rerun_if_changed("main.go")
            .with_rerun_if_changed("go.mod")
            .with_rerun_if_changed("go.sum");
        compile_go_to_wasm(config)
    } else {
        println!("cargo:rustc-env=FLUENTBASE_WASM_ARTIFACT_PATH=fallback.wasm");
        println!("cargo:warning=tinygo not found, wasm may be outdated");
    }
}
