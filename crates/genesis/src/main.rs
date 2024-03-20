use clap::Parser;
use fluentbase_genesis::devnet::devnet_genesis;
use std::fs;

/// Command line utility which generates genesis
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long, default_value = "")]
    dest_file_name: String,

    #[arg(long, default_value = "")]
    out_dir: String,

    #[arg(long, default_value = "")]
    genesis_type: String,
}

fn main() {
    let args = Args::parse();

    const DEVNET: &str = "devnet";
    const GENESIS_TYPES: &[&str] = &[DEVNET];

    let (genesis_type, genesis) = match args.genesis_type.as_str() {
        DEVNET => (args.genesis_type, devnet_genesis()),
        _ => {
            panic!("unsupported genesis type '{}'", args.genesis_type)
        }
    };

    let genesis_json = serde_json::to_string_pretty(&genesis).unwrap();
    let dest_file_name = if args.dest_file_name.is_empty() {
        format!("genesis-{}.json", genesis_type)
    } else {
        args.dest_file_name
    };
    let out_dir_with_slash = if args.out_dir.is_empty() {
        "".to_string()
    } else {
        format!("{}/", args.out_dir)
    };
    fs::write(
        format!("{}{}", out_dir_with_slash, dest_file_name),
        genesis_json,
    )
    .unwrap();
}
