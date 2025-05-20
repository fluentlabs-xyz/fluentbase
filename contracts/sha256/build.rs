use fluentbase_build::{compile_rust_to_wasm, Config};

fn main() {
    let config = Config::default()
        .with_rerun_if_changed("src")
        .with_rerun_if_changed("Cargo.toml");
    compile_rust_to_wasm(config)
}
