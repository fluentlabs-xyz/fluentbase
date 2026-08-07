// tests/abi_generation.rs
// ABI generation tests with struct support using insta snapshots

use fluentbase_build::solidity::generate_abi;
use insta::{assert_json_snapshot, Settings};
use std::fs;
use tempfile::TempDir;

/// Helper to create the project from fixture
fn fixture_to_project(fixture_name: &str) -> (TempDir, std::path::PathBuf) {
    let content = fs::read_to_string(format!("tests/fixtures/{fixture_name}.rs"))
        .expect("fixture should exist");

    let temp_dir = TempDir::new().expect("create temp dir");
    let project_path = temp_dir.path().to_path_buf();
    let src_dir = project_path.join("src");

    fs::create_dir_all(&src_dir).expect("create src dir");
    fs::write(src_dir.join("lib.rs"), content).expect("write lib.rs");

    (temp_dir, project_path)
}

#[test]
fn simple_struct_abi() {
    let (_temp, project) = fixture_to_project("simple_struct");
    let abi = generate_abi(&project).expect("generate ABI");

    // Use insta with settings to make snapshots more stable
    let mut settings = Settings::clone_current();
    settings.set_sort_maps(true); // Sort JSON keys for the stable output
    settings.bind(|| {
        assert_json_snapshot!(abi);
    });
}
#[test]
fn nested_struct_abi() {
    let (_temp, project) = fixture_to_project("nested_struct");
    let abi = generate_abi(&project).expect("generate ABI");

    // Use insta with settings to make snapshots more stable
    let mut settings = Settings::clone_current();
    settings.set_sort_maps(true); // Sort JSON keys for the stable output
    settings.bind(|| {
        assert_json_snapshot!(abi);
    });
}
#[test]
fn array_struct_abi() {
    let (_temp, project) = fixture_to_project("array_struct");
    let abi = generate_abi(&project).expect("generate ABI");

    // Use insta with settings to make snapshots more stable
    let mut settings = Settings::clone_current();
    settings.set_sort_maps(true); // Sort JSON keys for the stable output
    settings.bind(|| {
        assert_json_snapshot!(abi);
    });
}
#[test]
fn edge_cases_struct_abi() {
    let (_temp, project) = fixture_to_project("edge_cases_struct");
    let abi = generate_abi(&project).expect("generate ABI");

    // Use insta with settings to make snapshots more stable
    let mut settings = Settings::clone_current();
    settings.set_sort_maps(true); // Sort JSON keys for the stable output
    settings.bind(|| {
        assert_json_snapshot!(abi);
    });
}

/// Root file of a crate whose modules both declare a `Config` struct
const DUPLICATE_NAMES_ROOT: &str = r#"
#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{basic_entrypoint, derive::router, Address, SharedAPI, U256};

mod a;
mod b;

#[derive(Default)]
pub struct DuplicateNames<SDK> {
    sdk: SDK,
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> DuplicateNames<SDK> {
    pub fn set_a(&mut self, config: a::Config) -> U256 {
        config.value
    }

    pub fn set_b(&mut self, config: b::Config) -> Address {
        config.owner
    }
}

basic_entrypoint!(DuplicateNames);
"#;

const MODULE_A: &str = r#"
use fluentbase_sdk::{derive::Codec, U256};

#[derive(Codec, Debug, Clone)]
pub struct Config {
    pub value: U256,
    pub enabled: bool,
}
"#;

const MODULE_B: &str = r#"
use fluentbase_sdk::{derive::Codec, Address, U256};

#[derive(Codec, Debug, Clone)]
pub struct Config {
    pub owner: Address,
    pub limit: U256,
    pub label: String,
}
"#;

/// Write the duplicate-name crate, creating the module files in the given order
fn duplicate_names_project(module_order: &[&str]) -> (TempDir, std::path::PathBuf) {
    let temp_dir = TempDir::new().expect("create temp dir");
    let project_path = temp_dir.path().to_path_buf();
    let src_dir = project_path.join("src");
    fs::create_dir_all(&src_dir).expect("create src dir");

    for module in module_order {
        let content = match *module {
            "a" => MODULE_A,
            "b" => MODULE_B,
            other => panic!("unknown module {other}"),
        };
        fs::write(src_dir.join(format!("{module}.rs")), content).expect("write module");
    }
    fs::write(src_dir.join("lib.rs"), DUPLICATE_NAMES_ROOT).expect("write lib.rs");

    (temp_dir, project_path)
}

/// Same struct name in two modules must not collapse into one ABI definition
#[test]
fn duplicate_struct_names_resolve_per_module() {
    let (_temp, project) = duplicate_names_project(&["a", "b"]);
    let abi = generate_abi(&project).expect("generate ABI");

    let set_a = abi
        .iter()
        .find(|entry| entry["name"] == "setA")
        .expect("setA in ABI");
    let a_components = set_a["inputs"][0]["components"]
        .as_array()
        .expect("components for a::Config");
    assert_eq!(
        a_components
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["value", "enabled"]
    );

    let set_b = abi
        .iter()
        .find(|entry| entry["name"] == "setB")
        .expect("setB in ABI");
    let b_components = set_b["inputs"][0]["components"]
        .as_array()
        .expect("components for b::Config");
    assert_eq!(
        b_components
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["owner", "limit", "label"]
    );
}

/// The ABI must not depend on the order the source files happen to be enumerated in
#[test]
fn abi_is_independent_of_file_creation_order() {
    let orderings: [&[&str]; 4] = [&["a", "b"], &["b", "a"], &["a", "b"], &["b", "a"]];

    let artifacts = orderings
        .iter()
        .map(|order| {
            let (_temp, project) = duplicate_names_project(order);
            serde_json::to_string_pretty(&generate_abi(&project).expect("generate ABI"))
                .expect("serialize ABI")
        })
        .collect::<Vec<_>>();

    for artifact in &artifacts[1..] {
        assert_eq!(
            artifact, &artifacts[0],
            "ABI artifact changed with source file creation order"
        );
    }
}

/// A bare name that matches several modules is a hard error, not an arbitrary pick
#[test]
fn ambiguous_bare_struct_name_fails_the_build() {
    let (_temp, project) = duplicate_names_project(&["a", "b"]);
    let src_dir = project.join("src");
    fs::write(
        src_dir.join("lib.rs"),
        DUPLICATE_NAMES_ROOT
            .replace("config: a::Config", "config: Config")
            .replace("config: b::Config", "config: Config")
            .replace("config.owner", "config.value"),
    )
    .expect("rewrite lib.rs");

    let error = generate_abi(&project).expect_err("ambiguous struct name should fail");
    let error = error.to_string();
    assert!(error.contains("ambiguous"), "unexpected error: {error}");
    assert!(error.contains("a::Config"), "unexpected error: {error}");
    assert!(error.contains("b::Config"), "unexpected error: {error}");
}

#[test]
fn direct_impl_constructor() {
    let (_temp, project) = fixture_to_project("direct_impl_constructor");
    let abi = generate_abi(&project).expect("generate ABI");

    // Use insta with settings to make snapshots more stable
    let mut settings = Settings::clone_current();
    settings.set_sort_maps(true); // Sort JSON keys for the stable output
    settings.bind(|| {
        assert_json_snapshot!(abi);
    });
}

#[test]
fn trait_impl_constructor() {
    let (_temp, project) = fixture_to_project("trait_impl_constructor");
    let abi = generate_abi(&project).expect("generate ABI");

    // Use insta with settings to make snapshots more stable
    let mut settings = Settings::clone_current();
    settings.set_sort_maps(true); // Sort JSON keys for the stable output
    settings.bind(|| {
        assert_json_snapshot!(abi);
    });
}
