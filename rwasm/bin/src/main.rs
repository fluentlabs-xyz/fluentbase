mod types;

extern crate core;

use crate::types::FileFormat;
use clap::Parser;
use fluentbase_runtime::Runtime;
use fluentbase_rwasm::rwasm::{Compiler, CompilerConfig, FuncOrExport};
use std::{fs, path::Path};

/// Command line utility which takes input WAT/WASM file and converts it into RWASM
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long)]
    file_in_path: String,

    #[arg(long, default_value = "")]
    file_out_path: String,

    #[arg(long, default_value_t = false)]
    skip_translate_sections: bool,

    #[arg(long, default_value_t = false)]
    skip_type_check: bool,

    #[arg(long, default_value_t = false)]
    do_not_inject_fuel: bool,

    #[arg(long, default_value_t = false)]
    use_subroutine_router: bool,

    #[arg(long, default_value = "")]
    entry_fn_name: String,

    #[arg(long, default_value_t = false)]
    entry_fn_name_matches_file_in_name: bool,
}

fn main() {
    let args = Args::parse();
    let file_in_format: FileFormat;
    if args.file_in_path.ends_with(".wat") {
        file_in_format = FileFormat::Wat;
    } else if args.file_in_path.ends_with(".wasm") {
        file_in_format = FileFormat::Wasm;
    } else {
        panic!("only '.wat' and '.wasm' formats are supported")
    }

    let file_bytes = fs::read(args.file_in_path.clone()).unwrap();
    let wasm_binary: Vec<u8>;
    match file_in_format {
        FileFormat::Wat => {
            wasm_binary = wat::parse_bytes(&file_bytes).unwrap().to_vec();
        }
        FileFormat::Wasm => {
            wasm_binary = file_bytes;
        }
    }

    let import_linker = Runtime::<()>::new_linker();
    let mut compiler = Compiler::new_with_linker(
        &wasm_binary,
        CompilerConfig::default()
            .translate_sections(!args.skip_translate_sections)
            .type_check(!args.skip_type_check)
            .fuel_consume(!args.do_not_inject_fuel)
            .with_router(!args.use_subroutine_router),
        Some(&import_linker),
    )
    .unwrap();
    let file_in_path = Path::new(&args.file_in_path);
    let file_in_name = file_in_path.file_stem().unwrap().to_str().unwrap();
    let mut fn_idx = 0;
    let entry_fn_name = if args.entry_fn_name_matches_file_in_name {
        file_in_name.to_string()
    } else {
        args.entry_fn_name
    };
    if entry_fn_name != "" {
        let fn_name = Box::new(entry_fn_name);
        fn_idx = compiler
            .resolve_func_index(&FuncOrExport::Export(Box::leak(fn_name)))
            .unwrap()
            .unwrap();
    };
    compiler.translate(FuncOrExport::Func(fn_idx)).unwrap();
    let rwasm_binary = compiler.finalize().unwrap();
    let file_out_path;
    if args.file_out_path != "" {
        file_out_path = args.file_out_path;
    } else {
        file_out_path = format!(
            "{}/{}",
            file_in_path.parent().unwrap().to_str().unwrap(),
            format!("{}{}", file_in_name, types::RWASM_OUT_FILE_EXT)
        );
    }
    fs::write(file_out_path, rwasm_binary).unwrap();
}
