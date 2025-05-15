fn main() {
    #[cfg(feature = "enable_go")]
    {
        let config = fluentbase_build::Config::default()
            .with_rerun_if_changed("main.go")
            .with_rerun_if_changed("go.mod")
            .with_rerun_if_changed("go.sum");
        fluentbase_build::compile_go_to_wasm(config)
    }

    #[cfg(not(feature = "enable_go"))]
    {
        println!("cargo:warning=enable_go feature is not enabled, skipping Go compilation");
    }
}
