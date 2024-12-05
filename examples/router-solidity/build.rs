use std::{env, fs};

fn main() {
    // Get the manifest directory (where Cargo.toml is located)
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");

    // Get the artifacts directory from environment variable or use default
    let artifacts_dir = env::var("FLUENTBASE_CONTRACT_ARTIFACTS_DIR")
        .unwrap_or_else(|_| format!("{}/artifacts", manifest_dir));

    // Create the artifacts directory if it doesn't exist
    fs::create_dir_all(&artifacts_dir).expect("Failed to create artifacts directory");

    // Set environment variable for proc-macro
    println!(
        "cargo:rustc-env=FLUENTBASE_CONTRACT_ARTIFACTS_DIR={}",
        artifacts_dir
    );

    // Add rerun triggers
    println!("cargo:rerun-if-changed={}", artifacts_dir);
    println!("cargo:rerun-if-env-changed=FLUENTBASE_CONTRACT_ARTIFACTS_DIR");
}
