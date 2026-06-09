// Artefact fields unused while bootstrap.rs is a stub (Phase 2).
// Phase 3's revm deploy reads `init_bytecode` + `deployed_bytecode`
// for each contract.
#![allow(dead_code)]

use alloy_primitives::Bytes;
use eyre::WrapErr;
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize, Debug)]
struct ForgeArtefact {
    bytecode: ForgeBytecode,
    #[serde(rename = "deployedBytecode")]
    deployed_bytecode: ForgeBytecode,
}

#[derive(Deserialize, Debug)]
struct ForgeBytecode {
    object: String,
}

#[derive(Debug, Clone)]
pub struct ContractArtefact {
    pub init_bytecode: Bytes,
    pub deployed_bytecode: Bytes,
}

#[derive(Debug)]
pub struct Artefacts {
    pub staking: ContractArtefact,
    pub chain_config: ContractArtefact,
    pub staking_pool: ContractArtefact,
    pub system_reward: ContractArtefact,
    pub governance: ContractArtefact,
    pub liveness_slashing: ContractArtefact,
    pub mock_blend_token: ContractArtefact,
    pub bls_verifier: ContractArtefact,
}

fn load_one(path: &Path) -> eyre::Result<ContractArtefact> {
    let raw = std::fs::read_to_string(path)
        .wrap_err_with(|| format!("read forge artefact {}", path.display()))?;
    let parsed: ForgeArtefact = serde_json::from_str(&raw)
        .wrap_err_with(|| format!("parse forge artefact {}", path.display()))?;
    let init = Bytes::from(
        hex::decode(parsed.bytecode.object.trim_start_matches("0x"))
            .wrap_err("decode init bytecode")?,
    );
    let deployed = Bytes::from(
        hex::decode(parsed.deployed_bytecode.object.trim_start_matches("0x"))
            .wrap_err("decode deployed bytecode")?,
    );
    Ok(ContractArtefact {
        init_bytecode: init,
        deployed_bytecode: deployed,
    })
}

pub fn load(dir: &Path) -> eyre::Result<Artefacts> {
    Ok(Artefacts {
        staking: load_one(&dir.join("Staking.json"))?,
        chain_config: load_one(&dir.join("ChainConfig.json"))?,
        staking_pool: load_one(&dir.join("StakingPool.json"))?,
        system_reward: load_one(&dir.join("SystemReward.json"))?,
        governance: load_one(&dir.join("FluentGovernance.json"))?,
        liveness_slashing: load_one(&dir.join("LivenessSlashing.json"))?,
        mock_blend_token: load_one(&dir.join("MockBlendToken.json"))?,
        bls_verifier: load_one(&dir.join("BLS12381Verifier.json"))?,
    })
}
