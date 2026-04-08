use cargo_metadata::{CrateType, MetadataCommand, Package, PackageId, TargetKind};
use fluentbase_build::{docker, BuildArgs, DEFAULT_DOCKER_IMAGE, DEFAULT_DOCKER_TAG};
use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Default, Debug)]
struct PackagesResolver {
    manifest_dirs: Vec<PathBuf>,
    packages: Vec<Package>,
    workspace_members: HashSet<PackageId>,
}

impl PackagesResolver {
    fn find_packages(&mut self, contracts_dir: PathBuf) {
        println!("cargo:rerun-if-changed={}", contracts_dir.to_str().unwrap());
        let contracts_manifest_path = contracts_dir.join("Cargo.toml");
        let metadata = MetadataCommand::new()
            .manifest_path(&contracts_manifest_path)
            .exec()
            .unwrap();
        self.manifest_dirs.push(contracts_manifest_path);
        self.packages.extend_from_slice(&metadata.packages);
        for x in metadata.workspace_members {
            self.workspace_members.insert(x);
        }
    }
}

fn env_bool(name: &str) -> Option<bool> {
    env::var(name).ok().and_then(|value| {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        }
    })
}

fn contracts_build_args(fluentbase_root_dir: &Path) -> BuildArgs {
    let mut args = BuildArgs::default();

    let default_docker = env_bool("CI").unwrap_or(false);
    args.docker = env_bool("FLUENTBASE_CONTRACTS_DOCKER").unwrap_or(default_docker);

    args.docker_image = env::var("FLUENTBASE_BUILD_DOCKER_IMAGE")
        .unwrap_or_else(|_| DEFAULT_DOCKER_IMAGE.to_string());
    args.docker_tag =
        env::var("FLUENTBASE_BUILD_DOCKER_TAG").unwrap_or_else(|_| DEFAULT_DOCKER_TAG.to_string());
    args.mount_dir = Some(fluentbase_root_dir.to_path_buf());

    args.features = vec![];
    args.no_default_features = true;
    args.locked = true;

    if env::var("CARGO_FEATURE_DEBUG_PRINT").is_ok() {
        args.features.push("debug-print".to_string());
    }

    let has_contracts_cargo_config = fluentbase_root_dir
        .join("contracts/.cargo/config.toml")
        .exists()
        || fluentbase_root_dir.join("contracts/.cargo/config").exists();
    args.ignore_default_rust_flags = env_bool("FLUENTBASE_CONTRACTS_IGNORE_DEFAULT_RUST_FLAGS")
        .unwrap_or(has_contracts_cargo_config);

    args
}

fn map_path_for_docker(path: &Path, mount_dir: &Path) -> Option<PathBuf> {
    path.strip_prefix(mount_dir)
        .ok()
        .map(|relative| PathBuf::from("/workspace").join(relative))
}

fn run_workspace_build(
    build_args: &BuildArgs,
    workspace_manifest_path: &Path,
    target_dir: &Path,
    is_debug_profile: bool,
) {
    let work_dir = workspace_manifest_path.parent().unwrap();
    let mount_dir = build_args
        .mount_dir
        .as_deref()
        .unwrap_or_else(|| work_dir.parent().unwrap_or(work_dir));

    let effective_target_dir = if build_args.docker {
        map_path_for_docker(target_dir, mount_dir)
            .unwrap_or_else(|| PathBuf::from("/workspace/target/contracts"))
    } else {
        target_dir.to_path_buf()
    };

    let mut cargo_args = vec![
        "cargo".to_string(),
        "build".to_string(),
        "--target".to_string(),
        "wasm32-unknown-unknown".to_string(),
        "--target-dir".to_string(),
        effective_target_dir.to_string_lossy().to_string(),
        "--color=always".to_string(),
    ];

    if !is_debug_profile {
        cargo_args.push("--release".to_string());
    }

    if build_args.no_default_features {
        cargo_args.push("--no-default-features".to_string());
    }

    if !build_args.features.is_empty() {
        cargo_args.push("--features".to_string());
        cargo_args.push(build_args.features.join(","));
    }

    if build_args.locked {
        cargo_args.push("--locked".to_string());
    }

    let rust_flags = build_args.rust_flags();
    let env_vars = if rust_flags.is_empty() {
        vec![]
    } else {
        vec![("CARGO_ENCODED_RUSTFLAGS".to_string(), rust_flags)]
    };

    if build_args.docker {
        let image_ref = format!("{}:{}", build_args.docker_image, build_args.docker_tag);
        let image = docker::ensure_rust_image(&image_ref)
            .unwrap_or_else(|_| panic!("failed to ensure docker image {image_ref}"));

        let rust_toolchain = build_args.toolchain_version(work_dir);

        docker::run_in_docker(
            &image,
            &cargo_args,
            mount_dir,
            work_dir,
            &env_vars,
            &rust_toolchain,
        )
        .expect("WASM compilation failure in docker");
    } else {
        let mut command = Command::new("cargo");
        command.current_dir(work_dir).args(&cargo_args[1..]);
        for (key, value) in &env_vars {
            command.env(key, value);
        }

        let status = command
            .status()
            .expect("WASM compilation failure: failed to run cargo build");
        if !status.success() {
            panic!(
                "WASM compilation failure: cargo exited with code: {}",
                status.code().unwrap_or(1)
            );
        }
    }
}

fn main() {
    // Make sure we rerun the build if the feature has changed
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_STD");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_DEBUG_PRINT");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_WASMTIME");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_FLUENT_TESTNET");
    println!("cargo:rerun-if-env-changed=PROFILE");
    println!("cargo:rerun-if-env-changed=OPT_LEVEL");
    println!("cargo:rerun-if-env-changed=DEBUG");
    println!("cargo:rerun-if-env-changed=TARGET");
    println!("cargo:rerun-if-env-changed=CI");
    println!("cargo:rerun-if-env-changed=FLUENTBASE_CONTRACTS_DOCKER");
    println!("cargo:rerun-if-env-changed=FLUENTBASE_BUILD_DOCKER_IMAGE");
    println!("cargo:rerun-if-env-changed=FLUENTBASE_BUILD_DOCKER_TAG");
    println!("cargo:rerun-if-env-changed=FLUENTBASE_CONTRACTS_IGNORE_DEFAULT_RUST_FLAGS");

    let fluentbase_root_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let root_metadata = MetadataCommand::new()
        .manifest_path(fluentbase_root_dir.join("Cargo.toml"))
        .exec()
        .unwrap();
    let target2_dir: PathBuf = root_metadata.target_directory.join("contracts").into();

    let build_args = contracts_build_args(&fluentbase_root_dir);

    let mut packages_resolver = PackagesResolver::default();
    packages_resolver.find_packages(fluentbase_root_dir.join("contracts"));
    packages_resolver.find_packages(fluentbase_root_dir.join("examples"));

    let is_debug_profile = env::var("PROFILE").unwrap() == "debug";

    for contracts_manifest_path in &packages_resolver.manifest_dirs {
        run_workspace_build(
            &build_args,
            contracts_manifest_path,
            &target2_dir,
            is_debug_profile,
        );
    }

    let artifacts_dir = target2_dir
        .join("wasm32-unknown-unknown")
        .join(if is_debug_profile { "debug" } else { "release" });

    let mut paths: Vec<(String, PathBuf)> = Vec::new();

    for package in &packages_resolver.packages {
        if !packages_resolver.workspace_members.contains(&package.id) {
            continue;
        }
        for target in &package.targets {
            if !cfg!(feature = "svm") && target.name.contains("svm") {
                continue;
            }
            // Check for binary targets
            let is_bin = target.kind.contains(&TargetKind::Bin)
                && target.crate_types.contains(&CrateType::Bin);
            // Check for cdylib targets (also produces wasm binaries)
            let is_cdylib = target.kind.contains(&TargetKind::CDyLib)
                && target.crate_types.contains(&CrateType::CDyLib);

            if is_bin || is_cdylib {
                let mut path = artifacts_dir.clone();
                path.push(&target.name);
                path.set_extension("wasm");
                paths.push((target.name.clone(), path));
            }
        }
    }

    paths.sort_by(|a, b| a.0.cmp(&b.0));

    let mut code = vec![
        "pub struct BuildOutput {".to_string(),
        "    pub name: &'static str,".to_string(),
        "    pub wasm_bytecode: &'static [u8],".to_string(),
        "}".to_string(),
    ];
    for (name, path) in paths {
        let constant_name = name.to_uppercase().replace('-', "_");
        let path = path.to_str().unwrap();
        code.push(format!(
            "pub const {constant_name}: BuildOutput = BuildOutput {{"
        ));
        code.push(format!("    name: \"{name}\","));
        code.push(format!("    wasm_bytecode: include_bytes!(\"{path}\"),"));
        code.push("};".to_string());
    }
    let code = code.join("\n");
    let build_output_path = PathBuf::from(env::var("OUT_DIR").unwrap()).join("build_output.rs");

    fs::write(&build_output_path, code).unwrap();
}
