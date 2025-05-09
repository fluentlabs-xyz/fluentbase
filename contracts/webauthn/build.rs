fn main() {
    let profile = std::env::var("PROFILE").unwrap();
    println!("cargo:warning=Build profile: {}", profile);

    // Target triple
    let target = std::env::var("TARGET").unwrap();
    println!("cargo:warning=Target: {}", target);

    // Optimization level: "0", "1", "2", "3", "s", or "z"
    if let Ok(opt_level) = std::env::var("OPT_LEVEL") {
        println!("cargo:warning=Optimization level: {}", opt_level);
    }
    fluentbase_build::compile_rust_to_wasm(Default::default())
}
