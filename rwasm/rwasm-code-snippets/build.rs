// use fluentbase_rwasm::rwasm::{Compiler, CompilerConfig, FuncOrExport};
// use std::{env, fs, process::Command};

fn main() {
    // println!("cargo:rerun-if-changed=build.rs");

    // let cargo_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    // let out_dir = env::var("OUT_DIR").unwrap();
    //
    // let artefact_path = out_dir + "/../../../build";

    // let has_feature = env::var("CARGO_FEATURE_MEMORY_MSTORE").is_ok();
    // if has_feature {
    //     return;
    // }

    // Command::new("cargo")
    //     .args(&[
    //         "build",
    //         "--release",
    //         "--target=wasm32-unknown-unknown",
    //         "--features=memory_mstore",
    //     ])
    //     .spawn()
    //     .unwrap();

    // let mut dir = fs::read_dir(&artefact_path).unwrap();
    // let mut str = String::new();
    // while let Some(val) = dir.next() {
    //     str += val.unwrap().file_name().to_str().unwrap();
    //     str += "\n";
    // }
    // panic!("files: {}", str);

    // let target_arch = env::var("TARGET").unwrap();
    // if target_arch != "wasm32-unknown-unknown".to_string() {
    //     return;
    // }
    // let profile = env::var("PROFILE").unwrap();
    // if profile != "release".to_string() {
    //     return;
    // }
    // let artefact_path = out_dir + "/fluentbase_rwasm_code_snippets.wasm";
    // // panic!("{}", artefact_path);
    // let wasm_binary = fs::read(&artefact_path).expect("can't load wasm binary");
    //
    // let config = CompilerConfig::default()
    //     .fuel_consume(false)
    //     .translate_sections(false)
    //     .type_check(false);
    // let mut compiler =
    //     Compiler::new(wasm_binary.as_slice(), config).expect("failed to compile wasm binary");
    // let entry_func = compiler
    //     .resolve_any_export_func()
    //     .expect("there is no export function inside binary");
    // compiler
    //     .translate(FuncOrExport::Export(Box::leak(entry_func.clone())))
    //     .expect("failed to compile wasm binary");
    // let rwasm_binary = compiler.finalize().unwrap();
    //
    // fs::copy(
    //     &artefact_path,
    //     cargo_dir.clone() + "/bin/" + entry_func.as_ref() + ".wasm",
    // )
    // .unwrap();

    // panic!("haha: {}", out_dir);
}
