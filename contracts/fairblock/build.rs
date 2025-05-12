fn main() {
    let config = fluentbase_build::Config::default()
        .with_rerun_if_changed("main.go")
        .with_rerun_if_changed("go.mod")
        .with_rerun_if_changed("go.sum");
    fluentbase_build::compile_go_to_wasm(config)
}
