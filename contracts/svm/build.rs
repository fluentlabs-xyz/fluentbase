fn main() {
    // fluentbase_build::compile_rust_to_wasm(Default::default())
    let config = fluentbase_build::Config::default()
        .with_rerun_if_changed("src")
        .with_rerun_if_changed("Cargo.toml")
        .with_rerun_if_changed("../../crates/svm");
    fluentbase_build::compile_rust_to_wasm(config)
}
