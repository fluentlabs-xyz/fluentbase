// Artefact fields unused while bootstrap.rs is a stub (Phase 2).
// Phase 3's revm deploy reads `init_bytecode` + `deployed_bytecode`
// for each contract.
#![allow(dead_code)]

use crate::bootstrap::{STAKING_DPOS_ADDR, STAKING_ECONOMICS_ADDR};
use alloy_primitives::{Address, Bytes};
use eyre::WrapErr;
use serde::Deserialize;
use std::collections::HashMap;
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
    /// `{ "path/File.sol": { "LibName": [{start,length}] } }` — byte offsets of
    /// each unresolved library reference. Empty/absent for fully-linked or
    /// library-free bytecode.
    #[serde(rename = "linkReferences", default)]
    link_references: HashMap<String, HashMap<String, Vec<LinkRef>>>,
}

#[derive(Deserialize, Debug)]
struct LinkRef {
    start: usize,
    length: usize,
}

#[derive(Debug, Clone)]
pub struct ContractArtefact {
    pub init_bytecode: Bytes,
    pub deployed_bytecode: Bytes,
}

#[derive(Debug)]
pub struct Artefacts {
    pub staking: ContractArtefact,
    /// DELEGATECALL'd libraries `Staking` is linked against (deployed at
    /// `STAKING_DPOS_ADDR` / `STAKING_ECONOMICS_ADDR`). Forge leaves
    /// `__$<hash>$__` placeholders in `Staking`'s init+deployed bytecode keyed by
    /// library name in `linkReferences`; we link them ourselves at load.
    pub staking_dpos: ContractArtefact,
    pub staking_economics: ContractArtefact,
    pub chain_config: ContractArtefact,
    pub staking_pool: ContractArtefact,
    pub system_reward: ContractArtefact,
    pub governance: ContractArtefact,
    pub liveness_slashing: ContractArtefact,
    pub mock_blend_token: ContractArtefact,
    pub bls_verifier: ContractArtefact,
}

/// Splice each Solidity library placeholder (`__$<34 hex>$__`, 20 bytes) in a
/// bytecode hex string with its canonical address, driven by forge's
/// `linkReferences` (byte offsets keyed by library name). Forge auto-links to
/// CREATE2 addresses only at *deploy* time; the genesis-bootstrap deploys each
/// library at a fixed canonical address instead, so it links here before
/// decoding. Mapping by library NAME (not a blanket placeholder replace) is what
/// keeps the two distinct libraries — `StakingDpos` and `StakingEconomics` —
/// pointed at their own addresses; a blanket replace would collapse both onto one.
fn link_object(bc: &ForgeBytecode, libs: &[(&str, Address)]) -> eyre::Result<Bytes> {
    let mut chars = bc.object.trim_start_matches("0x").as_bytes().to_vec();
    for (file, per_lib) in &bc.link_references {
        for (lib_name, refs) in per_lib {
            let addr = libs
                .iter()
                .find(|(name, _)| name == lib_name)
                .map(|(_, addr)| *addr)
                .ok_or_else(|| eyre::eyre!("unmapped library {lib_name} referenced in {file}"))?;
            let addr_hex = hex::encode(addr.as_slice()); // 20 bytes → 40 hex chars
            for r in refs {
                let (s, e) = (r.start * 2, (r.start + r.length) * 2);
                chars[s..e].copy_from_slice(addr_hex.as_bytes());
            }
        }
    }
    let linked = String::from_utf8(chars).wrap_err("linked bytecode not utf8")?;
    Ok(Bytes::from(
        hex::decode(linked).wrap_err("decode bytecode")?,
    ))
}

fn load_one(path: &Path) -> eyre::Result<ContractArtefact> {
    load_one_linked(path, &[])
}

fn load_one_linked(path: &Path, libs: &[(&str, Address)]) -> eyre::Result<ContractArtefact> {
    let raw = std::fs::read_to_string(path)
        .wrap_err_with(|| format!("read forge artefact {}", path.display()))?;
    let parsed: ForgeArtefact = serde_json::from_str(&raw)
        .wrap_err_with(|| format!("parse forge artefact {}", path.display()))?;
    Ok(ContractArtefact {
        init_bytecode: link_object(&parsed.bytecode, libs)?,
        deployed_bytecode: link_object(&parsed.deployed_bytecode, libs)?,
    })
}

pub fn load(dir: &Path) -> eyre::Result<Artefacts> {
    let staking_libs: [(&str, Address); 2] = [
        ("StakingDpos", STAKING_DPOS_ADDR),
        ("StakingEconomics", STAKING_ECONOMICS_ADDR),
    ];
    Ok(Artefacts {
        staking: load_one_linked(&dir.join("Staking.json"), &staking_libs)?,
        staking_dpos: load_one(&dir.join("StakingDpos.json"))?,
        staking_economics: load_one(&dir.join("StakingEconomics.json"))?,
        chain_config: load_one(&dir.join("ChainConfig.json"))?,
        staking_pool: load_one(&dir.join("StakingPool.json"))?,
        system_reward: load_one(&dir.join("SystemReward.json"))?,
        governance: load_one(&dir.join("FluentGovernance.json"))?,
        liveness_slashing: load_one(&dir.join("LivenessSlashing.json"))?,
        mock_blend_token: load_one(&dir.join("MockBlendToken.json"))?,
        bls_verifier: load_one(&dir.join("BLS12381Verifier.json"))?,
    })
}
