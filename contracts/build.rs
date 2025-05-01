use std::{env, process::Command};

fn main() {
    if env::var("TARGET").unwrap() == "wasm32-unknown-unknown" {
        return;
    }

    let rust_flags = [
        "-C".to_string(),
        format!("link-arg=-zstack-size={}", 128 * 1024),
        "-C".to_string(),
        "panic=abort".to_string(),
        "-C".to_string(),
        "target-feature=+bulk-memory".to_string(),
    ];
    let rust_flags = rust_flags.join("\x1f");

    let build_arguments = vec![
        "build".to_string(),
        "--target".to_string(),
        "wasm32-unknown-unknown".to_string(),
        "--release".to_string(),
        "--manifest-path".to_string(),
        "./Cargo.toml".to_string(),
        "--target-dir".to_string(),
        "../target/target2".to_string(),
        "--no-default-features".to_string(),
    ];

    let status = Command::new("cargo")
        .env("CARGO_ENCODED_RUSTFLAGS", rust_flags)
        .args(build_arguments)
        .status()
        .expect("WASM compilation failure");
    if !status.success() {
        panic!(
            "WASM compilation failure with code: {}",
            status.code().unwrap_or(1)
        );
    }
}
