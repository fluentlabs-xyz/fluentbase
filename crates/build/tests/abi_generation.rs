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
