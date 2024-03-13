#![warn(unused_crate_dependencies)]

extern crate core;

use crate::types::FileFormat;
use clap::Parser;
use fluentbase_runtime::Runtime;
use log::debug;
use rwasm_codegen::{
    instruction::INSTRUCTION_SIZE_BYTES,
    Compiler,
    CompilerConfig,
    FuncOrExport,
    FUNC_SOURCE_MAP_ENTRYPOINT_IDX,
    FUNC_SOURCE_MAP_ENTRYPOINT_NAME,
};
use std::{fs, path::Path};

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
    print_rwasm_bytes: bool,

    #[arg(long, default_value_t = false)]
    gen_source_map: bool,

    #[arg(long, default_value_t = false)]
    do_not_translate_sections: bool,

    #[arg(long, default_value_t = false)]
    skip_type_check: bool,

    #[arg(long, default_value_t = false)]
    inject_fuel: bool,

    #[arg(long, default_value_t = false)]
    no_router: bool,

    #[arg(long, default_value = "")]
    entry_fn_name: String,

    #[arg(long, default_value = "")]
    restricted_fn_names: String,

    #[arg(long, default_value = "")]
    restricted_fn_name_prefixes: String,

    #[arg(long, default_value_t = false)]
    entry_fn_name_matches_file_in_name: bool,

    #[arg(long, default_value_t = false)]
    debug: bool,

    #[arg(long, default_value_t = false)]
    no_magic_prefix: bool,

    #[arg(long, default_value_t = false)]
    inject_init_bytecode: bool,

    #[arg(long, default_value_t = false)]
    retranslate_main: bool,
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

    let import_linker = Runtime::<()>::new_sovereign_linker();
    let mut compiler = Compiler::new_with_linker(
        &wasm_binary,
        CompilerConfig::default()
            .translate_sections(!args.do_not_translate_sections)
            .type_check(!args.skip_type_check)
            .fuel_consume(args.inject_fuel)
            .with_router(!args.no_router)
            .with_magic_prefix(!args.no_magic_prefix),
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
    if args.retranslate_main {
        compiler.translate(FuncOrExport::Func(fn_idx)).unwrap();
    }
    let func_source_maps = compiler.build_source_map();
    let entry_point_fn = &func_source_maps[0];
    debug!(
        "zero_fn_source_map name '{}' index '{}' pos '{}' len '{}'",
        entry_point_fn.fn_name,
        entry_point_fn.fn_index,
        entry_point_fn.position,
        entry_point_fn.length
    );
    let mut as_rust_vec: Vec<String> = vec![];
    let restricted_fn_names = args
        .restricted_fn_names
        .split(",")
        .collect::<Vec<&str>>()
        .iter()
        .map(|v| v.to_lowercase())
        .collect::<Vec<_>>();
    let restricted_fn_name_prefixes = args
        .restricted_fn_name_prefixes
        .split(",")
        .filter(|v| !v.is_empty())
        .collect::<Vec<&str>>()
        .iter()
        .map(|v| v.to_lowercase())
        .collect::<Vec<_>>();
    debug!("restricted_fn_names {:?}", restricted_fn_names.as_slice());
    debug!(
        "restricted_fn_name_prefixes {:?}",
        restricted_fn_name_prefixes.as_slice()
    );
    const FUNC_SYSTEM_PREFIX: &'static str = "$__";
    for func_source_map in &func_source_maps {
        debug!("func_source_map '{:?}'", func_source_map);
        let fn_name = func_source_map.fn_name.as_str();
        let fn_beginning = func_source_map.position;
        let fn_length = func_source_map.length;
        if fn_name == FUNC_SOURCE_MAP_ENTRYPOINT_NAME {
            let opcode = FUNC_SOURCE_MAP_ENTRYPOINT_IDX;
            as_rust_vec.push(format!("({opcode}, {fn_beginning}, {fn_length})"));
        } else {
            let mut is_restricted = fn_name.starts_with(FUNC_SYSTEM_PREFIX)
                || restricted_fn_names.contains(&fn_name.to_string());
            if !is_restricted && !restricted_fn_name_prefixes.is_empty() {
                is_restricted = restricted_fn_name_prefixes
                    .iter()
                    .find(|&p| fn_name.starts_with(p))
                    .is_some();
            };
            if !is_restricted {
                let fn_name = fn_name.to_uppercase();
                let fn_name_split = fn_name.split("_").collect::<Vec<_>>();
                let opcode_name = fn_name_split[fn_name_split.len() - 1];
                as_rust_vec.push(format!(
                    "(opcode::{opcode_name} as u32, {fn_beginning}, {fn_length})"
                ));
            }
        }
    }
    let rs_str = format!("[\n    {}\n]", as_rust_vec.join(",\n    "));
    let mut rwasm_binary = compiler.finalize().unwrap();
    // let init_bytecode_instruction_to_cut = 4; // redundant instruction inside init bytecode
    let init_bytecode = if args.inject_init_bytecode {
        rwasm_binary[entry_point_fn.position as usize * INSTRUCTION_SIZE_BYTES
            ..(entry_point_fn.position + entry_point_fn.length) as usize * INSTRUCTION_SIZE_BYTES]
            .to_vec()
    } else {
        vec![]
    };
    debug!(
        "extending rwasm_binary (byte len {}, instruction len {}) with init_bytecode (instruction position {} len {} fact len {})",
        rwasm_binary.len(),
        rwasm_binary.len() / INSTRUCTION_SIZE_BYTES,
        entry_point_fn.position,
        entry_point_fn.length,
        init_bytecode.len() / INSTRUCTION_SIZE_BYTES
    );
    if args.inject_init_bytecode {
        rwasm_binary.extend(&init_bytecode);
    }
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
    debug!(
        "rwasm_binary (byte len {}, instruction len {})",
        rwasm_binary.len(),
        rwasm_binary.len() / INSTRUCTION_SIZE_BYTES,
    );
    if args.print_rwasm_bytes {
        debug!("rwasm bytes: {:?}", rwasm_binary);
    }
    fs::write(rwasm_file_out_path, rwasm_binary).unwrap();

    if args.gen_source_map {
        let rs_source_map_file_out_path;
        if args.rs_file_out_path != "" {
            rs_source_map_file_out_path = args.rs_file_out_path;
        } else {
            rs_source_map_file_out_path = format!(
                "{}/{}",
                oud_dir_path,
                format!("{}{}", file_in_name, "_source_map.rs")
            );
        }
        fs::write(rs_source_map_file_out_path, rs_str).unwrap();
    }
}
#[ctor::ctor]
fn log_init() {
    let init_res =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
            .try_init();
    if let Err(e) = init_res {
        panic!("failed to init logger: {}", e.to_string());
    }
}
