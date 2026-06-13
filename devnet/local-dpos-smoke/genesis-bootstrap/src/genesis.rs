use alloy_genesis::Genesis;
use alloy_primitives::U256;
use eyre::WrapErr;
use std::collections::BTreeMap;

use crate::bootstrap::PredeployState;
use crate::keys::KeySet;

const BASE_GENESIS_JSON: &str = include_str!("../../../../crates/genesis/genesis-devnet.json");

pub fn assemble(chain_id: u64, keys: &KeySet, predeploy: PredeployState) -> eyre::Result<Genesis> {
    let mut g: Genesis =
        serde_json::from_str(BASE_GENESIS_JSON).wrap_err("parse base genesis-devnet.json")?;

    g.config.chain_id = chain_id;
    // Activate Paris hardfork at genesis. The named chainspecs ("dev",
    // "fluent-devnet" etc.) set `paris_block_and_final_difficulty` on the
    // static `ChainSpec`, but custom genesis JSON loaded via
    // `parse_genesis` goes through reth's `ChainSpec::from(Genesis)` —
    // which only activates Paris when `terminal_total_difficulty.is_some()`
    // (reth/crates/chainspec/src/spec.rs:838). Without this,
    // `is_paris_active_at_block` returns false → `BlockEnv.prevrandao = None`
    // (alloy-evm/src/eth/env.rs:90) → every system call (EIP-4788
    // beacon-root, `commitEpochCommittee`, `processBitmap`) panics on
    // `prevrandao().unwrap()` in `crates/revm/src/executor.rs:235`.
    g.config.terminal_total_difficulty = Some(U256::ZERO);
    g.config.terminal_total_difficulty_passed = true;
    g.extra_data = Default::default();
    // The fluentbase-genesis build.rs writes `SystemTime::now()` into the
    // embedded base JSON, so two `cargo run` invocations would otherwise
    // emit different `timestamp` fields and break the
    // deterministic-keys-and-genesis property smoke depends on. Pin to a
    // fixed epoch — only the runtime needs `timestamp <= block.timestamp`,
    // which any sensible value satisfies.
    g.timestamp = 0;

    for (addr, code) in predeploy.bytecode_by_address {
        let acct = g.alloc.entry(addr).or_default();
        acct.code = Some(code);
        if let Some(storage) = predeploy.storage_by_address.get(&addr) {
            let storage_btree: BTreeMap<_, _> = storage.iter().map(|(k, v)| (*k, *v)).collect();
            acct.storage = Some(storage_btree);
        }
        if let Some(balance) = predeploy.balance_by_address.get(&addr) {
            acct.balance = *balance;
        }
    }

    let wei_per_eth = U256::from(10u128.pow(18));
    for v in &keys.validators {
        let acct = g.alloc.entry(v.slasher.address()).or_default();
        acct.balance = wei_per_eth;
        let owner_acct = g.alloc.entry(v.l2_signer.address()).or_default();
        owner_acct.balance = wei_per_eth;
    }
    let gov_acct = g.alloc.entry(keys.governance_signer.address()).or_default();
    gov_acct.balance = wei_per_eth;

    Ok(g)
}
