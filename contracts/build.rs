use std::{env, process::Command};

fn main() {
    if env::var("TARGET").unwrap() == "wasm32-unknown-unknown" {
        return;
    }

    let mut args = vec![];
    args.push("build".to_string());
    args.push("--target".to_string());
    args.push("wasm32-unknown-unknown".to_string());
    // Always build release because wasm artifacts compiled in debug mode are too large (25+ MB
    // each). It's too slow to include ~500MB of debug symbols in the final artifact
    args.push("--release".to_string());
    args.push("--manifest-path".to_string());
    args.push("./Cargo.toml".to_string());
    args.push("--target-dir".to_string());
    args.push("../target/target2".to_string());
    args.push("--no-default-features".to_string());

    let rust_flags = [
        "-C".to_string(),
        format!("link-arg=-zstack-size={}", 128 * 1024),
        "-C".to_string(),
        "panic=abort".to_string(),
        "-C".to_string(),
        "target-feature=+bulk-memory".to_string(),
    ];
    let rust_flags = rust_flags.join("\x1f");

    let status = Command::new("cargo")
        .env("CARGO_ENCODED_RUSTFLAGS", rust_flags)
        .args(args)
        .status()
        .expect("WASM compilation failure");
    if !status.success() {
        panic!(
            "WASM compilation failure with code: {}",
            status.code().unwrap_or(1)
        );
    }
}
