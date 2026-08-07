//! Proves that path-form Solidity macros register the `.sol` file as a rustc
//! build input, so an incremental build in a reused target directory picks up
//! interface edits instead of keeping the stale expansion.

use alloy_sol_macro_input::SolInput;
use fluentbase_sdk_derive_core::sol_input::{to_rust_trait, to_sol_client};
use std::{fs, path::Path, path::PathBuf, process::Command};

/// The interface as it looks before the edit
const INTERFACE_V1: &str = r#"
interface IProgram {
    function transfer(address to, uint256 amount) external returns (bool);
}
"#;

/// The same interface after a change that moves every selector
const INTERFACE_V2: &str = r#"
interface IProgram {
    function transfer(address to, uint256 amount, bytes calldata data) external returns (bool);
}
"#;

/// Writes a `.sol` file into the per-test scratch directory
fn write_sol(name: &str, contents: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("sol_file_dependency");
    fs::create_dir_all(&dir).unwrap();

    let path = dir.join(name);
    fs::write(&path, contents).unwrap();
    path
}

/// Parses the path form of the macro input, the way `derive_solidity_trait!("x.sol")` does
fn parse_path_input(path: &Path) -> SolInput {
    // `{:?}` escapes the path into a Rust string literal, which is what the macro receives
    syn::parse_str(&format!("{:?}", path.to_str().unwrap())).unwrap()
}

#[test]
fn path_input_declares_the_sol_file_as_a_build_input() {
    let sol = write_sol("declared.sol", INTERFACE_V1);

    for generated in [
        to_rust_trait(parse_path_input(&sol)).unwrap().to_string(),
        to_sol_client(parse_path_input(&sol)).unwrap().to_string(),
    ] {
        assert!(
            generated.contains("include_str"),
            "expansion must declare the source file: {generated}"
        );
        assert!(
            generated.contains(sol.file_name().unwrap().to_str().unwrap()),
            "expansion must point at the file it read: {generated}"
        );
    }
}

#[test]
fn rustc_records_the_sol_file_in_dep_info() {
    let sol = write_sol("dep_info.sol", INTERFACE_V1);

    // Compile just the declaration the macro emits: the rest of the expansion needs
    // the SDK, while the dependency record is what incremental freshness turns on
    let generated = to_rust_trait(parse_path_input(&sol)).unwrap().to_string();
    let declaration = syn::parse_file(&generated)
        .unwrap()
        .items
        .into_iter()
        .find_map(|item| match item {
            syn::Item::Const(item) => Some(quote::quote!(#item).to_string()),
            _ => None,
        })
        .expect("expansion must contain the dependency declaration");

    let out_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("sol_file_dependency/dep_info");
    fs::create_dir_all(&out_dir).unwrap();
    let source = out_dir.join("lib.rs");
    fs::write(&source, declaration).unwrap();

    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let output = Command::new(rustc)
        .args([
            "--edition",
            "2021",
            "--crate-type",
            "lib",
            "--emit",
            "dep-info",
        ])
        .arg("--out-dir")
        .arg(&out_dir)
        .arg(&source)
        .output()
        .expect("failed to run rustc");
    assert!(
        output.status.success(),
        "emitted declaration must compile: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let dep_info = fs::read_to_string(out_dir.join("lib.d")).unwrap();
    assert!(
        dep_info.contains(sol.to_str().unwrap()),
        "rustc must list the .sol file as a build input: {dep_info}"
    );
}

#[test]
fn editing_the_sol_file_changes_the_expansion() {
    let sol = write_sol("reread.sol", INTERFACE_V1);
    let before = to_rust_trait(parse_path_input(&sol)).unwrap().to_string();

    fs::write(&sol, INTERFACE_V2).unwrap();
    let after = to_rust_trait(parse_path_input(&sol)).unwrap().to_string();

    assert_ne!(
        before, after,
        "a rebuild must reflect the edited interface, not the stale one"
    );
    assert!(
        !before.contains("data"),
        "unexpected v1 expansion: {before}"
    );
    assert!(after.contains("data"), "unexpected v2 expansion: {after}");
}
