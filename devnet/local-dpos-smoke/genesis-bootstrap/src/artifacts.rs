use alloy_primitives::Bytes;
use eyre::WrapErr;
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize, Debug)]
struct ForgeArtefact {
    bytecode: ForgeBytecode,
}

#[derive(Deserialize, Debug)]
struct ForgeBytecode {
    object: String,
}

#[derive(Debug)]
pub struct Artefacts {
    /// Prebuilt rWasm staking module, installed verbatim at `STAKING_ADDR` — never
    /// recompiled here. It was compiled with `compile_rwasm_maybe_system` bound to that
    /// address, so an in-process recompile would emit a fuel-metered module with a
    /// different code hash than the one the runtime-upgrade path produces. Provenance
    /// and the build command: `contracts/STAKING_ARTEFACT.md`.
    pub staking_rwasm: Bytes,
    /// Solidity init bytecode for the three contracts that stayed Solidity.
    pub staking_pool: Bytes,
    pub governance: Bytes,
    pub mock_blend_token: Bytes,
}

fn load_one(path: &Path) -> eyre::Result<Bytes> {
    let raw = std::fs::read_to_string(path)
        .wrap_err_with(|| format!("read forge artefact {}", path.display()))?;
    let parsed: ForgeArtefact = serde_json::from_str(&raw)
        .wrap_err_with(|| format!("parse forge artefact {}", path.display()))?;
    let decoded = hex::decode(parsed.bytecode.object.trim_start_matches("0x"))
        .wrap_err_with(|| format!("decode bytecode of {}", path.display()))?;
    Ok(Bytes::from(decoded))
}

/// Load ONLY the Governor's init bytecode — everything the `bare` arm installs.
///
/// Deliberately not `load`: that reads the staking rWasm module, and `bare` exists to
/// produce a chain without one.
pub fn load_governance(dir: &Path) -> eyre::Result<Bytes> {
    load_one(&dir.join("FluentGovernance.json"))
}

pub fn load(dir: &Path) -> eyre::Result<Artefacts> {
    let staking_path = dir.join("fluentbase_contracts_staking.rwasm");
    let staking_rwasm = std::fs::read(&staking_path)
        .wrap_err_with(|| format!("read staking rWasm module {}", staking_path.display()))?;
    Ok(Artefacts {
        staking_rwasm: Bytes::from(staking_rwasm),
        staking_pool: load_one(&dir.join("StakingPool.json"))?,
        governance: load_one(&dir.join("FluentGovernance.json"))?,
        mock_blend_token: load_one(&dir.join("MockBlendToken.json"))?,
    })
}
