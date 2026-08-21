// tests/abi_generation.rs
// ABI generation tests with struct support using insta snapshots

use fluentbase_build::solidity::{generate_abi, generate_interface, Abi};
use fluentbase_sdk_derive_core::{
    abi::structs::{StructRegistry, StructResolver},
    router::process_router_with_structs,
};
use insta::{assert_json_snapshot, Settings};
use quote::ToTokens;
use serde_json::Value;
use std::{fs, path::Path};
use syn::{visit::Visit, ItemImpl};
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

// ---------------------------------------------------------------------------------------------
// Selector agreement
//
// The selectors below are hard-coded from an independent implementation (`cast sig`), so they hold
// the router, the JSON ABI and the Solidity interface to the same signature rather than to each
// other.
// ---------------------------------------------------------------------------------------------

/// Selector the compiled router dispatches on, together with the signature it was hashed from
fn router_method(project: &Path, rust_name: &str) -> (String, String) {
    let entry_file = project.join("src").join("lib.rs");
    let source = fs::read_to_string(&entry_file).expect("read crate root");
    let ast = syn::parse_file(&source).expect("parse crate root");

    #[derive(Default)]
    struct RouterImpls(Vec<ItemImpl>);
    impl<'ast> Visit<'ast> for RouterImpls {
        fn visit_item_impl(&mut self, node: &'ast ItemImpl) {
            if node.attrs.iter().any(|attr| attr.path().is_ident("router")) {
                self.0.push(node.clone());
            }
            syn::visit::visit_item_impl(self, node);
        }
    }

    let mut impls = RouterImpls::default();
    impls.visit_file(&ast);

    let registry = StructRegistry::parse_crate(&entry_file).expect("parse structs");
    let resolver = StructResolver::registry(registry);

    for impl_block in impls.0 {
        let attr_tokens = match &impl_block
            .attrs
            .iter()
            .find(|attr| attr.path().is_ident("router"))
            .expect("router attribute")
            .meta
        {
            syn::Meta::List(list) => list.tokens.clone(),
            _ => Default::default(),
        };

        let router =
            process_router_with_structs(attr_tokens, impl_block.to_token_stream(), &resolver)
                .expect("process router");

        for method in router.available_methods() {
            if method.parsed_signature().rust_name() == rust_name {
                let selector = method
                    .function_id()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                return (method.signature().to_string(), format!("0x{selector}"));
            }
        }
    }

    panic!("method `{rust_name}` not found in any router");
}

/// Canonical signature of a published ABI entry, rebuilt straight from the JSON
///
/// Deliberately independent of the ABI types themselves: this is the expansion any caller would
/// perform on the artifact before hashing it.
fn published_signature(abi: &Abi, name: &str) -> String {
    fn canonical(param: &Value) -> String {
        let ty = param["type"].as_str().expect("parameter type");
        let (base, suffix) = match ty.find('[') {
            Some(index) => ty.split_at(index),
            None => (ty, ""),
        };

        if base != "tuple" {
            return ty.to_string();
        }

        let components = param["components"]
            .as_array()
            .expect("tuple parameter without components")
            .iter()
            .map(canonical)
            .collect::<Vec<_>>()
            .join(",");

        format!("({components}){suffix}")
    }

    let entry = abi
        .iter()
        .find(|entry| entry["name"] == name)
        .unwrap_or_else(|| panic!("`{name}` in ABI"));

    let inputs = entry["inputs"]
        .as_array()
        .expect("inputs")
        .iter()
        .map(canonical)
        .collect::<Vec<_>>()
        .join(",");

    format!("{name}({inputs})")
}

/// Every published entry agrees with the router on both the signature and its selector
fn assert_selectors_agree(project: &Path, expected: &[(&str, &str, &str)]) {
    let abi = generate_abi(project).expect("generate ABI");

    for (rust_name, signature, selector) in expected {
        let (router_signature, router_selector) = router_method(project, rust_name);

        assert_eq!(
            router_signature, *signature,
            "router signature of `{rust_name}`"
        );
        assert_eq!(
            router_selector, *selector,
            "router selector of `{rust_name}`"
        );

        let sol_name = signature.split('(').next().expect("function name");
        assert_eq!(
            published_signature(&abi, sol_name),
            *signature,
            "published signature of `{rust_name}`"
        );
    }
}

/// Nested struct parameters hash the same in the router and in the artifacts
#[test]
fn nested_struct_selectors_agree_with_the_router() {
    let (_temp, project) = fixture_to_project("nested_struct");

    assert_selectors_agree(
        &project,
        &[
            (
                "create_user",
                "createUser((address,(string,uint256,bool),uint256))",
                "0x01ef28b9",
            ),
            (
                "submit_order",
                "submitOrder((uint256,(address,(string,uint256,bool),uint256),\
                 (address,(string,uint256,bool),uint256),(uint256,address,uint256),uint8))",
                "0x6d4684cd",
            ),
            (
                "match_order",
                "matchOrder((address,(string,uint256,bool),uint256),\
                 (address,(string,uint256,bool),uint256),(uint256,address,uint256))",
                "0x47eab435",
            ),
            ("get_user", "getUser(address)", "0x6f77926b"),
        ],
    );

    // The interface a caller compiles against declares the same struct
    let abi = generate_abi(&project).expect("generate ABI");
    let interface = generate_interface("Nested", &abi).expect("generate interface");
    assert!(
        interface
            .contains(
                "function createUser(User calldata user) external payable returns (address _0);"
            ),
        "unexpected interface: {interface}"
    );
}

/// Struct arrays expand to their components instead of hashing as `tuple[]`
#[test]
fn struct_array_selectors_agree_with_the_router() {
    let (_temp, project) = fixture_to_project("array_struct");

    assert_selectors_agree(
        &project,
        &[
            (
                "add_pools",
                "addPools((address,address,uint256,uint256,uint256)[])",
                "0xd252d7cc",
            ),
            (
                "execute_route",
                "executeRoute(((address,address,uint256,uint256,uint256)[],address[],uint256))",
                "0xb73fabb0",
            ),
            (
                "update_reserves",
                "updateReserves((address,address,uint256,uint256,uint256)[],uint256[])",
                "0xcaf046bf",
            ),
            (
                "apply_batch_update",
                "applyBatchUpdate((((address,address,uint256,uint256,uint256),uint256)[],uint256))",
                "0xbc61d897",
            ),
        ],
    );
}

/// Module-qualified structs resolve to their own definition on both sides
#[test]
fn module_qualified_struct_selectors_agree_with_the_router() {
    let (_temp, project) = duplicate_names_project(&["a", "b"]);

    assert_selectors_agree(
        &project,
        &[
            ("set_a", "setA((uint256,bool))", "0xb6ea7d04"),
            ("set_b", "setB((address,uint256,string))", "0xdc78fda8"),
        ],
    );
}

/// A custom selector that no longer matches the published ABI stops the build
#[test]
fn selector_that_diverges_from_the_abi_fails_the_build() {
    let (_temp, project) = duplicate_names_project(&["a", "b"]);
    fs::write(
        project.join("src").join("lib.rs"),
        DUPLICATE_NAMES_ROOT.replace(
            "    pub fn set_a(",
            "    #[function_id(\"renameMe((uint256,bool))\")]\n    pub fn set_a(",
        ),
    )
    .expect("rewrite lib.rs");

    let error = format!(
        "{:#}",
        generate_abi(&project).expect_err("a router selector the ABI cannot reproduce should fail")
    );
    assert!(error.contains("0x410cd56e"), "unexpected error: {error}");
    assert!(error.contains("ABI migration"), "unexpected error: {error}");
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
