// Artefact fields unused while bootstrap.rs is a stub (Phase 2).
// Phase 3's revm deploy reads `init_bytecode` + `deployed_bytecode`
// for each contract.
#![allow(dead_code)]

use crate::bootstrap::STAKING_DPOS_ADDR;
use alloy_primitives::{Address, Bytes};
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
    /// DELEGATECALL'd library `Staking` is linked against (deployed at
    /// `STAKING_DPOS_ADDR`). Forge leaves `__$<hash>$__` placeholders in
    /// `Staking`'s init+deployed bytecode; we link them ourselves at load.
    pub staking_dpos: ContractArtefact,
    pub chain_config: ContractArtefact,
    pub staking_pool: ContractArtefact,
    pub system_reward: ContractArtefact,
    pub governance: ContractArtefact,
    pub liveness_slashing: ContractArtefact,
    pub mock_blend_token: ContractArtefact,
    pub bls_verifier: ContractArtefact,
}

/// Replace every Solidity library placeholder `__$<34 hex>$__` (exactly 40 chars)
/// in a bytecode hex string with `lib_addr` (40 hex chars). Forge auto-links the
/// `StakingDpos` library to its CREATE2 address only at *deploy* time; the
/// genesis-bootstrap deploys it at a fixed canonical address instead, so it must
/// perform the link itself before decoding `Staking`'s bytecode.
fn link_libraries(obj: &str, lib_addr: Address) -> String {
    let addr_hex = hex::encode(lib_addr.as_slice()); // 20 bytes → 40 hex chars
    let mut out = obj.to_string();
    while let Some(pos) = out.find("__$") {
        out.replace_range(pos..pos + 40, &addr_hex);
    }
    out
}

fn load_one(path: &Path) -> eyre::Result<ContractArtefact> {
    load_one_linked(path, None)
}

fn load_one_linked(path: &Path, lib: Option<Address>) -> eyre::Result<ContractArtefact> {
    let raw = std::fs::read_to_string(path)
        .wrap_err_with(|| format!("read forge artefact {}", path.display()))?;
    let parsed: ForgeArtefact = serde_json::from_str(&raw)
        .wrap_err_with(|| format!("parse forge artefact {}", path.display()))?;
    let link = |obj: &str| -> String {
        let trimmed = obj.trim_start_matches("0x");
        match lib {
            Some(addr) => link_libraries(trimmed, addr),
            None => trimmed.to_string(),
        }
    };
    let init = Bytes::from(hex::decode(link(&parsed.bytecode.object)).wrap_err("decode init bytecode")?);
    let deployed = Bytes::from(
        hex::decode(link(&parsed.deployed_bytecode.object)).wrap_err("decode deployed bytecode")?,
    );
    Ok(ContractArtefact {
        init_bytecode: init,
        deployed_bytecode: deployed,
    })
}

pub fn load(dir: &Path) -> eyre::Result<Artefacts> {
    Ok(Artefacts {
        staking: load_one_linked(&dir.join("Staking.json"), Some(STAKING_DPOS_ADDR))?,
        staking_dpos: load_one(&dir.join("StakingDpos.json"))?,
        chain_config: load_one(&dir.join("ChainConfig.json"))?,
        staking_pool: load_one(&dir.join("StakingPool.json"))?,
        system_reward: load_one(&dir.join("SystemReward.json"))?,
        governance: load_one(&dir.join("FluentGovernance.json"))?,
        liveness_slashing: load_one(&dir.join("LivenessSlashing.json"))?,
        mock_blend_token: load_one(&dir.join("MockBlendToken.json"))?,
        bls_verifier: load_one(&dir.join("BLS12381Verifier.json"))?,
    })
}
