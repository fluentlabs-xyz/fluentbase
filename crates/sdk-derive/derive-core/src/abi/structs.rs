//! Resolution of `#[derive(Codec)]` structs used as contract parameters
//!
//! A Rust struct in a contract signature carries no field information at the point where a
//! parameter is converted to its Solidity type: `rust_to_sol` only sees the type path. Leaving it
//! that way makes the router selector fall out of an empty tuple while artifact generation later
//! publishes the real components, so callers hash a different signature than the deployed router
//! dispatches on. Both sides therefore resolve components through this module before any selector
//! is calculated.
//!
//! Discovery walks the crate's module tree starting from `lib.rs`/`main.rs` and follows `mod`
//! declarations in source order, so the registry never depends on filesystem enumeration order.
//! Structs are keyed by their module-qualified path (`types::Config`) rather than the bare
//! identifier, so two modules declaring the same struct name no longer overwrite each other.

use crate::abi::{error::ABIError, parameter::Parameter};
use quote::ToTokens;
use std::{
    cell::OnceCell,
    collections::{BTreeMap, HashSet},
    env, fmt,
    path::{Path, PathBuf},
};
use syn::{
    parse::Parser, parse_file, punctuated::Punctuated, Attribute, DeriveInput, Item, ItemMod,
    ItemStruct, Meta, Path as SynPath, Token,
};

/// Crate root candidates, in the order cargo itself would pick them
const CRATE_ROOT_CANDIDATES: [&str; 4] = ["src/lib.rs", "src/main.rs", "lib.rs", "main.rs"];

/// Registry of `#[derive(Codec)]` structs found in a contract crate.
///
/// Keys are module-qualified paths relative to the crate root: `Config` for a struct in the root
/// module, `types::Config` for one declared inside `mod types`.
#[derive(Default, Clone)]
pub struct StructRegistry {
    // A `BTreeMap` keeps iteration - and therefore lookups and diagnostics - deterministic.
    structs: BTreeMap<String, Vec<DeriveInput>>,
}

// `syn::DeriveInput` only implements `Debug` under the `extra-traits` feature, so the registry
// reports the paths it holds instead of the definitions themselves.
impl fmt::Debug for StructRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StructRegistry")
            .field("structs", &self.structs.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl StructRegistry {
    /// Parse every `#[derive(Codec)]` struct reachable from a crate entry file
    ///
    /// # Arguments
    /// * `entry_file` - Path to the crate root (`src/lib.rs` or `src/main.rs`)
    pub fn parse_crate(entry_file: &Path) -> Result<Self, ABIError> {
        let mod_dir = entry_file
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();

        let mut walker = CrateWalker::default();
        walker.walk_file(entry_file, &mod_dir, &mut Vec::new())?;
        Ok(walker.registry)
    }

    /// Parse the crate rooted at a package directory, locating its entry file
    pub fn parse_package(package_dir: &Path) -> Result<Self, ABIError> {
        let entry_file = CRATE_ROOT_CANDIDATES
            .iter()
            .map(|candidate| package_dir.join(candidate))
            .find(|candidate| candidate.is_file())
            .ok_or_else(|| {
                ABIError::StructResolution(format!(
                    "no crate root (lib.rs or main.rs) found in {}",
                    package_dir.display()
                ))
            })?;

        Self::parse_crate(&entry_file)
    }

    /// Resolve a type path exactly as it was written in a contract signature
    ///
    /// # Arguments
    /// * `path` - Type path as written, e.g. `Config`, `types::Config` or `crate::types::Config`
    /// * `scope` - Module the path was written in (`""` for the crate root)
    ///
    /// # Returns
    /// * `Ok(Some((path, def)))` - The matched module-qualified path and its definition
    /// * `Ok(None)` - No `Codec` struct of this crate matches
    /// * `Err(_)` - The name is ambiguous, so picking a definition would be arbitrary
    pub fn resolve(
        &self,
        path: &str,
        scope: &str,
    ) -> Result<Option<(&str, &DeriveInput)>, ABIError> {
        let segments = split_path(path);
        if segments.is_empty() {
            return Ok(None);
        }

        // Innermost scope first, mirroring Rust name resolution closely enough for the module
        // layouts a contract can express.
        let scope_segments = split_path(scope);
        for depth in (0..=scope_segments.len()).rev() {
            let mut candidate = scope_segments[..depth].to_vec();
            candidate.extend_from_slice(&segments);
            if let Some((key, definitions)) = self.structs.get_key_value(&candidate.join("::")) {
                return single_definition(key, definitions).map(Some);
            }
        }

        // Fall back to a unique suffix match anywhere in the crate, which is what lets a bare
        // `Config` in a signature find `types::Config` while a duplicated name is rejected.
        let matches = self
            .structs
            .keys()
            .filter(|key| ends_with_segments(key, &segments))
            .collect::<Vec<_>>();

        match matches.as_slice() {
            [] => Ok(None),
            [only] => single_definition(only, &self.structs[*only]).map(Some),
            ambiguous => Err(ABIError::StructResolution(format!(
                "type `{path}` is ambiguous: it matches {}. \
                 Use the module-qualified path in the contract signature.",
                quoted_list(ambiguous.iter().map(|key| key.as_str()))
            ))),
        }
    }

    fn insert(&mut self, module: &[String], item: &ItemStruct) {
        let key = qualify(module, &item.ident.to_string());
        let derive_input = item_struct_to_derive_input(item);
        let encoded = derive_input.to_token_stream().to_string();

        let definitions = self.structs.entry(key).or_default();
        // `cfg`-gated definitions can legitimately repeat a name within one module; only
        // textually different ones are a genuine conflict.
        if definitions
            .iter()
            .any(|existing| existing.to_token_stream().to_string() == encoded)
        {
            return;
        }
        definitions.push(derive_input);
    }
}

/// Where struct definitions come from when a parameter has to be resolved.
///
/// Resolution is lazy: a contract whose methods only take primitive types never reaches the
/// filesystem, so the cost is paid exactly by the contracts that need it.
pub enum StructResolver {
    /// Structs of the crate currently being compiled, parsed on demand from `CARGO_MANIFEST_DIR`.
    ///
    /// This is what the `#[router]`, `#[constructor]` and `#[client]` macros use: the selector
    /// they bake into the dispatch table has to be computed from the same components the build
    /// tooling later publishes.
    CrateSources(OnceCell<Result<StructRegistry, String>>),
    /// A registry the caller has already parsed, used by the build tooling.
    Registry(StructRegistry),
}

impl Default for StructResolver {
    fn default() -> Self {
        Self::crate_sources()
    }
}

impl StructResolver {
    /// Resolver that parses the crate being compiled when a struct is first encountered
    #[must_use]
    pub fn crate_sources() -> Self {
        Self::CrateSources(OnceCell::new())
    }

    /// Resolver backed by an already parsed registry
    #[must_use]
    pub fn registry(registry: StructRegistry) -> Self {
        Self::Registry(registry)
    }

    /// Returns the struct definitions, parsing the crate sources on first use
    pub fn structs(&self) -> Result<&StructRegistry, ABIError> {
        match self {
            Self::Registry(registry) => Ok(registry),
            Self::CrateSources(cell) => cell
                .get_or_init(|| {
                    let package_dir = env::var("CARGO_MANIFEST_DIR").map_err(|_| {
                        "CARGO_MANIFEST_DIR is not set, so the crate sources cannot be located"
                            .to_string()
                    })?;
                    StructRegistry::parse_package(Path::new(&package_dir))
                        .map_err(|error| error.to_string())
                })
                .as_ref()
                .map_err(|error| {
                    ABIError::StructResolution(format!(
                        "cannot resolve struct parameters: {error}. Annotate the method with \
                         #[function_id(\"...\")] to pin its selector explicitly."
                    ))
                }),
        }
    }
}

impl Parameter {
    /// Path of the struct this parameter holds, with any array suffixes removed
    #[must_use]
    pub fn struct_path(&self) -> Option<&str> {
        self.internal_type
            .strip_prefix("struct ")
            .map(|name| strip_array_suffixes(name).trim())
    }

    /// Whether this parameter (or any component of it) is a struct whose fields are still unknown
    #[must_use]
    pub fn has_unresolved_struct(&self) -> bool {
        let unresolved = self.struct_path().is_some() && self.components.is_none();

        unresolved
            || self
                .components
                .iter()
                .flatten()
                .any(Parameter::has_unresolved_struct)
    }

    /// Fill in the components of every struct this parameter is built from
    ///
    /// `scope` is the module whose namespace the parameter's type was written in: the crate root
    /// for contract signatures, and the struct's own module for the components of an expanded
    /// struct.
    pub fn resolve_structs(
        &mut self,
        structs: &StructRegistry,
        scope: &str,
    ) -> Result<(), ABIError> {
        self.resolve_structs_inner(structs, scope, &mut Vec::new())
    }

    fn resolve_structs_inner(
        &mut self,
        structs: &StructRegistry,
        scope: &str,
        expanding: &mut Vec<String>,
    ) -> Result<(), ABIError> {
        // Components of an expanded struct are written in that struct's module, not in `scope`
        let mut nested_scope = scope.to_string();
        let mut expanded_path = None;

        if let Some(name) = self.struct_path().map(str::to_string) {
            let (path, definition) = structs.resolve(&name, scope)?.ok_or_else(|| {
                ABIError::StructResolution(format!(
                    "struct `{name}` appears in the contract ABI but has no `#[derive(Codec)]` \
                     definition in this crate; the generated ABI would have no components for it"
                ))
            })?;

            // A struct that contains itself has no finite ABI signature, and expanding it would
            // otherwise recurse until the stack runs out.
            if expanding.iter().any(|open| open == path) {
                return Err(ABIError::StructResolution(format!(
                    "struct `{path}` is recursive, so it has no Solidity ABI representation"
                )));
            }

            nested_scope = module_of(path).to_string();
            let path = path.to_string();
            let expanded = Parameter::from_derive_input(definition)?;
            self.components = expanded.components;
            expanding.push(path.clone());
            expanded_path = Some(path);
        }

        for component in self.components.iter_mut().flatten() {
            component.resolve_structs_inner(structs, &nested_scope, expanding)?;
        }

        if expanded_path.is_some() {
            expanding.pop();
        }

        Ok(())
    }
}

/// Walks a crate's module tree, collecting structs with `#[derive(Codec)]`
#[derive(Default)]
struct CrateWalker {
    registry: StructRegistry,
    visited: HashSet<PathBuf>,
}

impl CrateWalker {
    /// Parse one source file and descend into the modules it declares
    ///
    /// `mod_dir` is the directory holding the child modules of this file.
    fn walk_file(
        &mut self,
        file: &Path,
        mod_dir: &Path,
        module: &mut Vec<String>,
    ) -> Result<(), ABIError> {
        // `#[path]` attributes make it possible to reach the same file twice
        let marker = file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
        if !self.visited.insert(marker) {
            return Ok(());
        }

        let content = std::fs::read_to_string(file).map_err(|error| {
            ABIError::StructResolution(format!("failed to read {}: {error}", file.display()))
        })?;
        let ast = parse_file(&content).map_err(|error| {
            ABIError::StructResolution(format!(
                "failed to parse Rust file {}: {error}",
                file.display()
            ))
        })?;

        self.walk_items(&ast.items, mod_dir, module)
    }

    fn walk_items(
        &mut self,
        items: &[Item],
        mod_dir: &Path,
        module: &mut Vec<String>,
    ) -> Result<(), ABIError> {
        for item in items {
            match item {
                Item::Struct(item_struct) if has_codec_derive(&item_struct.attrs) => {
                    self.registry.insert(module, item_struct);
                }
                Item::Mod(item_mod) => self.walk_module(item_mod, mod_dir, module)?,
                _ => {}
            }
        }
        Ok(())
    }

    fn walk_module(
        &mut self,
        item_mod: &ItemMod,
        mod_dir: &Path,
        module: &mut Vec<String>,
    ) -> Result<(), ABIError> {
        let name = item_mod.ident.to_string();

        // Inline module: its own children live one directory deeper
        if let Some((_, items)) = &item_mod.content {
            let child_dir = mod_dir.join(&name);
            module.push(name);
            let result = self.walk_items(items, &child_dir, module);
            module.pop();
            return result;
        }

        let Some(file) = module_file(item_mod, mod_dir, &name) else {
            // Modules we cannot locate (generated sources, platform-gated files) simply
            // contribute no structs; rustc reports the missing file if it actually matters.
            eprintln!(
                "Warning: could not locate source file for module `{name}` in {}",
                mod_dir.display()
            );
            return Ok(());
        };

        let child_dir = child_mod_dir(&file);
        module.push(name);
        let result = self.walk_file(&file, &child_dir, module);
        module.pop();
        result
    }
}

/// Locate the file backing `mod name;`, honoring `#[path = "..."]`
fn module_file(item_mod: &ItemMod, mod_dir: &Path, name: &str) -> Option<PathBuf> {
    if let Some(path) = path_attribute(&item_mod.attrs) {
        let candidate = mod_dir.join(path);
        return candidate.is_file().then_some(candidate);
    }

    [
        mod_dir.join(format!("{name}.rs")),
        mod_dir.join(name).join("mod.rs"),
    ]
    .into_iter()
    .find(|candidate| candidate.is_file())
}

/// Extract the value of a `#[path = "..."]` attribute
fn path_attribute(attrs: &[Attribute]) -> Option<String> {
    attrs.iter().find_map(|attr| match &attr.meta {
        Meta::NameValue(name_value) if name_value.path.is_ident("path") => {
            match &name_value.value {
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(literal),
                    ..
                }) => Some(literal.value()),
                _ => None,
            }
        }
        _ => None,
    })
}

/// Directory holding the child modules of a file
///
/// `src/a.rs` -> `src/a`, `src/a/mod.rs` -> `src/a`
fn child_mod_dir(file: &Path) -> PathBuf {
    let parent = file.parent().unwrap_or_else(|| Path::new("."));
    match file.file_stem().and_then(|stem| stem.to_str()) {
        Some("mod") | None => parent.to_path_buf(),
        Some(stem) => parent.join(stem),
    }
}

/// Returns true if attributes contain `#[derive(Codec)]`
fn has_codec_derive(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| match &attr.meta {
        Meta::List(list) if list.path.is_ident("derive") => {
            let derives = Punctuated::<SynPath, Token![,]>::parse_terminated
                .parse2(list.tokens.clone())
                .ok();

            derives
                .map(|d| d.iter().any(|p| p.is_ident("Codec")))
                .unwrap_or(false)
        }
        _ => false,
    })
}

/// Convert `ItemStruct` to `DeriveInput`
fn item_struct_to_derive_input(item: &ItemStruct) -> DeriveInput {
    DeriveInput {
        attrs: item.attrs.clone(),
        vis: item.vis.clone(),
        ident: item.ident.clone(),
        generics: item.generics.clone(),
        data: syn::Data::Struct(syn::DataStruct {
            struct_token: item.struct_token,
            fields: item.fields.clone(),
            semi_token: item.semi_token,
        }),
    }
}

/// Split a type path into segments, dropping the path qualifiers a signature may carry
fn split_path(path: &str) -> Vec<&str> {
    path.split("::")
        .map(str::trim)
        .filter(|segment| !segment.is_empty() && !matches!(*segment, "crate" | "self" | "super"))
        .collect()
}

/// Whether a registry key ends with the requested path segments
fn ends_with_segments(key: &str, segments: &[&str]) -> bool {
    let key_segments = key.split("::").collect::<Vec<_>>();
    key_segments.len() >= segments.len()
        && key_segments[key_segments.len() - segments.len()..] == *segments
}

/// Build a module-qualified path for a struct declared in `module`
fn qualify(module: &[String], name: &str) -> String {
    if module.is_empty() {
        name.to_string()
    } else {
        format!("{}::{name}", module.join("::"))
    }
}

/// Module part of a qualified path (`types::Config` -> `types`, `Config` -> `""`)
fn module_of(path: &str) -> &str {
    path.rfind("::").map_or("", |index| &path[..index])
}

fn quoted_list<'a>(items: impl Iterator<Item = &'a str>) -> String {
    items
        .map(|item| format!("`{item}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Reject paths that hold more than one distinct definition rather than picking one
fn single_definition<'a>(
    path: &'a str,
    definitions: &'a [DeriveInput],
) -> Result<(&'a str, &'a DeriveInput), ABIError> {
    match definitions {
        [only] => Ok((path, only)),
        _ => Err(ABIError::StructResolution(format!(
            "`{path}` has {} conflicting `#[derive(Codec)]` definitions; \
             the generated ABI would depend on which one is picked",
            definitions.len()
        ))),
    }
}

/// Strip every array suffix from a type name (`Cell[][]` -> `Cell`, `Item[3][2]` -> `Item`)
fn strip_array_suffixes(name: &str) -> &str {
    match name.find('[') {
        Some(index) => &name[..index],
        None => name,
    }
}

#[cfg(test)]
mod tests {
    mod parse {
        use crate::abi::structs::StructRegistry;
        use std::fs;
        use tempfile::TempDir;

        /// Helper to create a temporary crate with the given root file content
        fn crate_with_root(content: &str) -> (TempDir, std::path::PathBuf) {
            let temp_dir = TempDir::new().unwrap();
            let file_path = temp_dir.path().join("lib.rs");
            fs::write(&file_path, content).unwrap();
            (temp_dir, file_path)
        }

        fn parse(content: &str) -> (TempDir, StructRegistry) {
            let (temp_dir, file_path) = crate_with_root(content);
            let registry = StructRegistry::parse_crate(&file_path).unwrap();
            (temp_dir, registry)
        }

        /// Module-qualified paths held by the registry, in sorted order
        fn paths(registry: &StructRegistry) -> Vec<&str> {
            registry.structs.keys().map(String::as_str).collect()
        }

        #[test]
        fn test_parse_simple_struct_with_codec() {
            let content = r#"
use fluentbase_sdk::codec::Codec;
use fluentbase_sdk::U256;

#[derive(Codec, Debug, Clone)]
pub struct TestStruct {
    pub field1: U256,
    pub field2: bool,
    pub field3: Address,
}

#[derive(Debug, Clone)]
pub struct StructWithoutCodec {
    pub field1: u32,
}

#[derive(Codec)]
pub struct AnotherCodecStruct {
    pub value: U256,
}
"#;

            let (_temp_dir, structs) = parse(content);

            // Should find exactly the 2 structs with Codec
            assert_eq!(paths(&structs), vec!["AnotherCodecStruct", "TestStruct"]);

            // Verify the TestStruct has correct fields
            let (_, test_struct) = structs.resolve("TestStruct", "").unwrap().unwrap();
            if let syn::Data::Struct(data) = &test_struct.data {
                let field_names: Vec<String> = data
                    .fields
                    .iter()
                    .filter_map(|f| f.ident.as_ref().map(|i| i.to_string()))
                    .collect();

                assert_eq!(field_names, vec!["field1", "field2", "field3"]);
            } else {
                panic!("Expected struct data");
            }
        }

        #[test]
        fn test_parse_nested_structs() {
            let content = r#"
use fluentbase_sdk::codec::Codec;
use fluentbase_sdk::U256;

mod inner {
    use super::*;

    #[derive(Codec)]
    pub struct InnerStruct {
        pub value: U256,
    }
}

#[derive(Codec, Debug)]
pub struct OuterStruct {
    pub inner: inner::InnerStruct,
    pub data: U256,
}
"#;

            let (_temp_dir, structs) = parse(content);

            // Should find both inner and outer structs, the inner one module-qualified
            assert_eq!(paths(&structs), vec!["OuterStruct", "inner::InnerStruct"]);

            // Both the qualified and the bare name resolve to the same definition
            assert!(structs.resolve("inner::InnerStruct", "").unwrap().is_some());
            assert_eq!(
                structs.resolve("InnerStruct", "").unwrap().unwrap().0,
                "inner::InnerStruct"
            );
        }

        #[test]
        fn test_empty_file() {
            let content = r#"
// Empty file with no structs
use fluentbase_sdk::codec::Codec;
"#;

            let (_temp_dir, structs) = parse(content);

            assert!(paths(&structs).is_empty());
        }

        #[test]
        fn test_struct_with_unnamed_fields() {
            let content = r#"
use fluentbase_sdk::codec::Codec;
use fluentbase_sdk::U256;

#[derive(Codec)]
pub struct TupleStruct(pub U256, pub bool);

#[derive(Codec)]
pub struct UnitStruct;
"#;

            let (_temp_dir, structs) = parse(content);

            assert_eq!(paths(&structs), vec!["TupleStruct", "UnitStruct"]);
        }

        #[test]
        fn test_duplicate_names_in_different_modules_are_kept_apart() {
            let content = r#"
use fluentbase_sdk::codec::Codec;

mod a {
    #[derive(Codec)]
    pub struct Config {
        pub value: U256,
    }
}

mod b {
    #[derive(Codec)]
    pub struct Config {
        pub owner: Address,
        pub flag: bool,
    }
}
"#;

            let (_temp_dir, structs) = parse(content);

            assert_eq!(paths(&structs), vec!["a::Config", "b::Config"]);

            // Qualified paths resolve to their own definition
            let (path, definition) = structs.resolve("a::Config", "").unwrap().unwrap();
            assert_eq!(path, "a::Config");
            assert_eq!(field_names(definition), vec!["value"]);

            let (path, definition) = structs.resolve("b::Config", "").unwrap().unwrap();
            assert_eq!(path, "b::Config");
            assert_eq!(field_names(definition), vec!["owner", "flag"]);

            // A bare duplicate name is rejected instead of silently picking one
            let error = structs.resolve("Config", "").unwrap_err().to_string();
            assert!(error.contains("ambiguous"), "unexpected error: {error}");
            assert!(error.contains("a::Config"), "unexpected error: {error}");
            assert!(error.contains("b::Config"), "unexpected error: {error}");
        }

        #[test]
        fn test_bare_name_resolves_within_its_own_module_scope() {
            let content = r#"
mod a {
    #[derive(Codec)]
    pub struct Config {
        pub value: U256,
    }
}

mod b {
    #[derive(Codec)]
    pub struct Config {
        pub owner: Address,
    }
}
"#;

            let (_temp_dir, structs) = parse(content);

            // The same bare name resolves differently depending on the module it was written in
            assert_eq!(
                structs.resolve("Config", "a").unwrap().unwrap().0,
                "a::Config"
            );
            assert_eq!(
                structs.resolve("Config", "b").unwrap().unwrap().0,
                "b::Config"
            );
        }

        #[test]
        fn test_unknown_type_resolves_to_none() {
            let content = r#"
#[derive(Codec)]
pub struct Known {
    pub value: U256,
}
"#;

            let (_temp_dir, structs) = parse(content);

            assert!(structs.resolve("Unknown", "").unwrap().is_none());
        }

        #[test]
        fn test_walks_file_modules_and_ignores_undeclared_files() {
            let temp_dir = TempDir::new().unwrap();
            let src = temp_dir.path();

            fs::write(src.join("lib.rs"), "mod types;\nmod nested;\n").unwrap();
            fs::write(
                src.join("types.rs"),
                "#[derive(Codec)] pub struct Config { pub value: U256 }",
            )
            .unwrap();
            fs::create_dir(src.join("nested")).unwrap();
            fs::write(src.join("nested").join("mod.rs"), "mod deep;").unwrap();
            fs::write(
                src.join("nested").join("deep.rs"),
                "#[derive(Codec)] pub struct Config { pub owner: Address }",
            )
            .unwrap();
            // Not declared by any `mod`, so it is not part of the crate
            fs::write(
                src.join("orphan.rs"),
                "#[derive(Codec)] pub struct Orphan { pub value: U256 }",
            )
            .unwrap();

            let structs = StructRegistry::parse_crate(&src.join("lib.rs")).unwrap();

            assert_eq!(
                paths(&structs),
                vec!["nested::deep::Config", "types::Config"]
            );
        }

        #[test]
        fn test_module_path_attribute_is_honored() {
            let temp_dir = TempDir::new().unwrap();
            let src = temp_dir.path();

            fs::write(
                src.join("lib.rs"),
                "#[path = \"custom/location.rs\"]\nmod types;\n",
            )
            .unwrap();
            fs::create_dir(src.join("custom")).unwrap();
            fs::write(
                src.join("custom").join("location.rs"),
                "#[derive(Codec)] pub struct Config { pub value: U256 }",
            )
            .unwrap();

            let structs = StructRegistry::parse_crate(&src.join("lib.rs")).unwrap();

            assert_eq!(paths(&structs), vec!["types::Config"]);
        }

        #[test]
        fn test_conflicting_definitions_under_one_path_are_rejected() {
            let content = r#"
#[cfg(feature = "a")]
#[derive(Codec)]
pub struct Config {
    pub value: U256,
}

#[cfg(not(feature = "a"))]
#[derive(Codec)]
pub struct Config {
    pub owner: Address,
}
"#;

            let (_temp_dir, structs) = parse(content);

            let error = structs.resolve("Config", "").unwrap_err().to_string();
            assert!(error.contains("conflicting"), "unexpected error: {error}");
        }

        #[test]
        fn test_package_root_is_located_from_the_package_directory() {
            let temp_dir = TempDir::new().unwrap();
            let package = temp_dir.path();
            fs::create_dir(package.join("src")).unwrap();
            fs::write(
                package.join("src").join("lib.rs"),
                "#[derive(Codec)] pub struct Config { pub value: U256 }",
            )
            .unwrap();

            let structs = StructRegistry::parse_package(package).unwrap();
            assert_eq!(paths(&structs), vec!["Config"]);

            let missing = TempDir::new().unwrap();
            let error = StructRegistry::parse_package(missing.path())
                .unwrap_err()
                .to_string();
            assert!(error.contains("no crate root"), "unexpected error: {error}");
        }

        fn field_names(definition: &syn::DeriveInput) -> Vec<String> {
            match &definition.data {
                syn::Data::Struct(data) => data
                    .fields
                    .iter()
                    .filter_map(|f| f.ident.as_ref().map(|i| i.to_string()))
                    .collect(),
                _ => panic!("Expected struct data"),
            }
        }
    }

    mod resolve {
        use crate::abi::{parameter::Parameter, structs::StructRegistry};
        use std::fs;
        use tempfile::TempDir;

        /// Helper to build a registry from crate root source
        fn registry(content: &str) -> (TempDir, StructRegistry) {
            let temp_dir = TempDir::new().unwrap();
            let file_path = temp_dir.path().join("lib.rs");
            fs::write(&file_path, content).unwrap();
            let registry = StructRegistry::parse_crate(&file_path).unwrap();
            (temp_dir, registry)
        }

        /// A struct parameter as it comes out of a contract signature: named, but componentless
        fn struct_parameter(name: &str, internal_type: &str, ty: &str) -> Parameter {
            Parameter {
                internal_type: internal_type.to_string(),
                ty: ty.to_string(),
                name: name.to_string(),
                components: None,
            }
        }

        fn component_names(param: &Parameter) -> Vec<&str> {
            param
                .components
                .as_ref()
                .expect("components")
                .iter()
                .map(|component| component.name.as_str())
                .collect()
        }

        #[test]
        fn test_resolve_simple_struct_parameter() {
            let (_temp, structs) = registry(
                r#"
#[derive(Codec)]
pub struct SlippageParams {
    pub amount_in: U256,
    pub reserve_in: U256,
    pub reserve_out: U256,
    pub fee_rate: U256,
}
"#,
            );

            let mut param = struct_parameter("params", "struct SlippageParams", "tuple");
            assert!(param.has_unresolved_struct());

            param.resolve_structs(&structs, "").unwrap();

            assert!(!param.has_unresolved_struct());
            assert_eq!(
                component_names(&param),
                vec!["amount_in", "reserve_in", "reserve_out", "fee_rate"]
            );
            for component in param.components.as_ref().unwrap() {
                assert_eq!(component.ty, "uint256");
            }
            assert_eq!(
                param.get_canonical_type().unwrap(),
                "(uint256,uint256,uint256,uint256)"
            );
        }

        #[test]
        fn test_resolve_uses_qualified_path_from_signature() {
            let (_temp, structs) = registry(
                r#"
mod a {
    #[derive(Codec)]
    pub struct Config {
        pub value: U256,
    }
}

mod b {
    #[derive(Codec)]
    pub struct Config {
        pub owner: Address,
        pub flag: bool,
    }
}
"#,
            );

            let mut param = struct_parameter("config", "struct b::Config", "tuple");
            param.resolve_structs(&structs, "").unwrap();

            assert_eq!(component_names(&param), vec!["owner", "flag"]);
        }

        #[test]
        fn test_resolve_rejects_ambiguous_bare_name() {
            let (_temp, structs) = registry(
                r#"
mod a {
    #[derive(Codec)]
    pub struct Config {
        pub value: U256,
    }
}

mod b {
    #[derive(Codec)]
    pub struct Config {
        pub owner: Address,
    }
}
"#,
            );

            let mut param = struct_parameter("config", "struct Config", "tuple");
            let error = param.resolve_structs(&structs, "").unwrap_err().to_string();
            assert!(error.contains("ambiguous"), "unexpected error: {error}");
        }

        #[test]
        fn test_resolve_rejects_unknown_struct() {
            let (_temp, structs) = registry("#[derive(Codec)] pub struct Known { pub v: U256 }");

            let mut param = struct_parameter("value", "struct Missing", "tuple");
            let error = param.resolve_structs(&structs, "").unwrap_err().to_string();
            assert!(
                error.contains("no `#[derive(Codec)]` definition"),
                "unexpected error: {error}"
            );
        }

        #[test]
        fn test_resolve_nested_struct_in_its_own_module() {
            // `Outer` lives in `a` and refers to a bare `Inner` that also exists in `b`;
            // the nested lookup must stay inside `a`.
            let (_temp, structs) = registry(
                r#"
mod a {
    #[derive(Codec)]
    pub struct Inner {
        pub value: U256,
    }

    #[derive(Codec)]
    pub struct Outer {
        pub inner: Inner,
    }
}

mod b {
    #[derive(Codec)]
    pub struct Inner {
        pub owner: Address,
        pub flag: bool,
    }
}
"#,
            );

            let mut param = struct_parameter("outer", "struct a::Outer", "tuple");
            param.resolve_structs(&structs, "").unwrap();

            let inner = &param.components.as_ref().unwrap()[0];
            assert_eq!(component_names(inner), vec!["value"]);
            assert_eq!(param.get_canonical_type().unwrap(), "((uint256))");
        }

        #[test]
        fn test_resolve_struct_array_parameters() {
            let (_temp, structs) = registry(
                r#"
#[derive(Codec)]
pub struct Item {
    pub id: U256,
    pub owner: Address,
}
"#,
            );

            let mut dynamic = struct_parameter("dynamic", "struct Item[]", "tuple[]");
            let mut fixed = struct_parameter("fixed", "struct Item[3]", "tuple[3]");

            dynamic.resolve_structs(&structs, "").unwrap();
            fixed.resolve_structs(&structs, "").unwrap();

            for param in [&dynamic, &fixed] {
                assert_eq!(component_names(param), vec!["id", "owner"]);
            }
            assert_eq!(dynamic.get_canonical_type().unwrap(), "(uint256,address)[]");
            assert_eq!(fixed.get_canonical_type().unwrap(), "(uint256,address)[3]");
        }

        #[test]
        fn test_recursive_struct_is_rejected() {
            let (_temp, structs) = registry(
                r#"
#[derive(Codec)]
pub struct Node {
    pub children: Vec<Node>,
}
"#,
            );

            let mut param = struct_parameter("node", "struct Node", "tuple");
            let error = param.resolve_structs(&structs, "").unwrap_err().to_string();
            assert!(error.contains("recursive"), "unexpected error: {error}");
        }
    }
}
