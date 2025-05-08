use cargo_metadata::camino::Utf8PathBuf;
use fluentbase_build::{build_wasm_program, Config};
use std::fs;

pub fn build_all_examples() -> Vec<(String, Utf8PathBuf)> {
    let mut dirs: Vec<String> = Vec::new();
    fs::read_dir("../examples")
        .expect("failed to read directory")
        .for_each(|entry| {
            let path = entry.expect("failed to read entry").path();
            if path.is_dir() {
                let program = path.to_str().expect("failed to convert path to string");
                dirs.push(program.to_string());
            }
        });

    let mut available_example_contracts = Vec::new();
    for dir in dirs {
        // build wasm bytecode for each contract in contracts/**
        let config = Config::default().with_cargo_manifest_dir(dir);
        let (target_name, wasm_path) = build_wasm_program(config).unwrap();
        println!(
            "compiled example contract \"{}\" to {}",
            target_name, wasm_path
        );
        available_example_contracts.push((target_name, wasm_path));
    }

    available_example_contracts
}

fn main() {
    let all_examples = build_all_examples();
    assert!(
        !all_examples.is_empty(),
        "examples folders should not be empty"
    );
}
