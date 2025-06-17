mod config;
mod utils;
use clap::{Parser, ValueEnum};
pub use config::*;
use std::path::PathBuf;
pub use utils::*;

const DEFAULT_DOCKER_TAG: &str = concat!("v", env!("CARGO_PKG_VERSION"));

const BUILD_TARGET: &str = "wasm32-unknown-unknown";
const HELPER_TARGET_SUBDIR: &str = "wasm-compilation";

#[derive(Clone, ValueEnum, Debug, PartialEq)]
pub enum Artifact {
    /// Solidity ABI JSON file
    Abi,
    /// Solidity interface (.sol) file
    Solidity,
    /// Contract verification metadata
    Metadata,
}

#[derive(Clone, ValueEnum, Debug, Default)]
pub enum WarningLevel {
    /// Show all warning messages (default)
    #[default]
    All,
    /// Suppress non-essential warnings
    Minimal,
}

/// Compile a Fluent smart contract to WASM/rWASM
#[derive(Clone, Parser, Debug)]
pub struct BuildArgs {
    #[arg(
        long,
        action,
        help = "Run compilation using a Docker container for reproducible builds"
    )]
    pub docker: bool,

    #[arg(
        long,
        help = "The ghcr.io/fluentlabs/fluent-build image tag to use when building with Docker",
        default_value = DEFAULT_DOCKER_TAG
    )]
    pub tag: String,

    #[arg(
        long,
        action,
        value_delimiter = ',',
        help = "Space or comma separated list of features to activate"
    )]
    pub features: Vec<String>,

    #[arg(long, action, help = "Do not activate the `default` feature")]
    pub no_default_features: bool,

    #[arg(long, action, help = "Assert that `Cargo.lock` will remain unchanged")]
    pub locked: bool,

    #[arg(
        long,
        action,
        value_delimiter = ',',
        help = "Space or comma separated list of extra flags to invoke `rustc` with"
    )]
    pub rustflags: Vec<String>,

    #[arg(
        short,
        long,
        alias = "out-dir",
        help = "Copy the compiled artifacts to this directory",
        default_value = "out"
    )]
    pub output: PathBuf,

    #[arg(
        short = 'g',
        long,
        value_enum,
        value_delimiter = ',',
        help = "Additional artifacts to generate beyond WASM/rWASM"
    )]
    pub generate: Vec<Artifact>,

    #[arg(
        long,
        action,
        help = "Force build even with uncommitted changes (dirty git state)"
    )]
    pub force: bool,

    #[arg(
        long,
        value_enum,
        default_value = "all",
        help = "Control warning message verbosity"
    )]
    pub warning_level: WarningLevel,

    #[arg(
        long,
        alias = "workspace-dir",
        action,
        help = "The top level directory to be used in the docker invocation"
    )]
    pub workspace_directory: Option<PathBuf>,
}

impl Default for BuildArgs {
    fn default() -> Self {
        Self {
            docker: true,
            tag: DEFAULT_DOCKER_TAG.to_string(),
            features: vec![],
            no_default_features: true,
            locked: true,
            rustflags: vec![],
            output: PathBuf::from("out"),
            generate: vec![],
            force: false,
            warning_level: WarningLevel::All,
            workspace_directory: None,
        }
    }
}
