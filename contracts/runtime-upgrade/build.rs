use fluentbase_build::{build_with_args, Artifact, BuildArgs};
use std::path::PathBuf;

fn main() {
    // Build local artifacts
    let build_args = BuildArgs {
        docker: true,
        mount_dir: Some(PathBuf::from("../../")),
        generate: vec![Artifact::Abi, Artifact::Solidity],
        output_path: Some("./".to_string()),
        ..Default::default()
    };
    build_with_args(".", build_args);
    // Build out artifacts
    let build_args = BuildArgs {
        docker: true,
        mount_dir: Some(PathBuf::from("../../")),
        generate: vec![
            Artifact::Rwasm,
            Artifact::Abi,
            Artifact::Metadata,
            Artifact::Foundry,
            Artifact::Solidity,
        ],
        output_path: Some("../out/{contract_name}/".to_string()),
        ..Default::default()
    };
    build_with_args(".", build_args);
}
