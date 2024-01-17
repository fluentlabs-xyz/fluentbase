extern crate core;

use crate::{opcodes::OPCODE_NAME_TO_NUMBER, types::FileFormat};
use clap::Parser;
use fluentbase_runtime::Runtime;
use fluentbase_rwasm::rwasm::{Compiler, CompilerConfig, FuncOrExport};
use std::{fs, io::BufRead, path::Path};

mod opcodes;
mod types;

/// Command line utility which takes input WAT/WASM file and converts it into RWASM
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long)]
    file_in_path: String,

    #[arg(long, default_value = "")]
    rwasm_file_out_path: String,

    #[arg(long, default_value = "")]
    rs_file_out_path: String,

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

    #[arg(long, default_value = "")]
    entry_fn_name_beginnings_for: String,

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
            .with_router(!args.use_subroutine_router)
            .with_magic_prefix(false),
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
    let mut as_rust_vec: Vec<String> = vec![];
    let mut as_json_arr: Vec<String> = vec![];
    if args.entry_fn_name_beginnings_for != "" {
        for fn_name in args.entry_fn_name_beginnings_for.split(" ") {
            let fn_name = Box::new(fn_name.to_string());
            let fn_idx = compiler
                .resolve_func_index(&FuncOrExport::Export(Box::leak(fn_name.clone())))
                .unwrap_or(None);
            let fn_beginning = if fn_idx.is_some() {
                *compiler
                    .resolve_func_beginning(fn_idx.unwrap())
                    .unwrap_or(&0)
            } else {
                0
            };
            println!(
                "fn_name '{fn_name}' idx '{:?}' begins at '{:?}'",
                fn_idx, fn_beginning
            );
            let fn_name = fn_name.to_uppercase();
            let fn_name_split = fn_name.split("_").collect::<Vec<_>>();
            let opcode_name = fn_name_split[fn_name_split.len() - 1];
            as_rust_vec.push(format!("(opcode::{opcode_name}, {fn_beginning})"));
            let opcode_number = OPCODE_NAME_TO_NUMBER.get(opcode_name).unwrap();
            as_json_arr.push(format!("[{opcode_number},{fn_beginning}]"));
        }
    }
    let json_str = format!("[{}]", as_json_arr.join(","));
    let rs_str = format!("[{}]", as_rust_vec.join(","));
    println!("rust [(opcode::NAME, FN_ENTRY_OFFSET)]: \n[{}]", rs_str);
    let rwasm_binary = compiler.finalize().unwrap();
    let rwasm_file_out_path;
    let oud_dir_path = file_in_path.parent().unwrap().to_str().unwrap();
    if args.rwasm_file_out_path != "" {
        rwasm_file_out_path = args.rwasm_file_out_path;
    } else {
        rwasm_file_out_path = format!(
            "{}/{}",
            oud_dir_path,
            format!("{}{}", file_in_name, types::RWASM_OUT_FILE_EXT)
        );
    }
    fs::write(rwasm_file_out_path, rwasm_binary).unwrap();

    let rs_file_out_path;
    if args.rs_file_out_path != "" {
        rs_file_out_path = args.rs_file_out_path;
    } else {
        rs_file_out_path = format!("{}/{}", oud_dir_path, format!("{}{}", file_in_name, ".rs"));
    }
    fs::write(rs_file_out_path, json_str).unwrap();
}
