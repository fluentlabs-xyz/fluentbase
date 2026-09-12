use crate::utils::download_and_cache_genesis_verified;
use alloy_primitives::{b256, hex};
use fluentbase_genesis::local_genesis_from_file;
use fluentbase_release_verify::ReleaseAsset;
use reth_chainspec::{
    make_genesis_header, BaseFeeParams, BaseFeeParamsKind, Chain, ChainHardforks, ChainSpec,
    EthereumHardfork, EthereumHardforks, ForkCondition, Hardfork, DEV_HARDFORKS,
};
use reth_cli::chainspec::{parse_genesis, ChainSpecParser};
use reth_primitives_traits::SealedHeader;
use reth_revm::primitives::U256;
use std::sync::{Arc, LazyLock};
use tracing::warn;

// Genesis assets for the built-in networks.
//
// Each entry names a GitHub release asset and pins the SHA-256 of the exact `.json.gz` published
// there. The pins are checked in addition to the detached OpenPGP signature; when a tag is bumped,
// re-pin with `shasum -a 256 <asset>` after confirming the asset's `.asc` verifies against the
// release key pinned in `fluentbase-release-verify`.

/// Genesis asset for Fluent Devnet (GitHub releases).
fn devnet_genesis() -> ReleaseAsset {
    ReleaseAsset::genesis("v0.5.7", None)
        .expect("built-in devnet release asset must be valid")
        .with_sha256(hex!(
            "91b9a427805d45dd14e46a0cd517bcc85f350fe7dfc38fa96f6ff0ebf5e864da"
        ))
}

/// Genesis asset for Fluent Testnet (GitHub releases).
fn testnet_genesis() -> ReleaseAsset {
    ReleaseAsset::genesis("v0.3.4-dev", None)
        .expect("built-in testnet release asset must be valid")
        .with_sha256(hex!(
            "8cd30358c5664375e6739bc48302445e7ee10fd0158bedb788505e5c590983bd"
        ))
}

/// Genesis asset for Fluent Mainnet (GitHub releases).
fn mainnet_genesis() -> ReleaseAsset {
    ReleaseAsset::genesis("v1.0.0", Some("mainnet"))
        .expect("built-in mainnet release asset must be valid")
        .with_sha256(hex!(
            "72cb4b3b7b15de952bd1094281a1f2430cb711bc473a0520f92aa3e2b1bdb643"
        ))
}

/// Every genesis asset a built-in network can be started from.
///
/// Kept as a table so tests can assert the fail-closed guarantees hold for all of them; keep it in
/// sync when a network is added.
#[cfg(test)]
pub(crate) fn built_in_genesis_assets() -> Vec<(&'static str, ReleaseAsset)> {
    vec![
        ("fluent-devnet", devnet_genesis()),
        ("fluent-testnet", testnet_genesis()),
        ("fluent-mainnet", mainnet_genesis()),
    ]
}

pub const FLUENT_LOCALNET_CHAIN_ID: u64 = 1337;
pub const FLUENT_DEVNET_CHAIN_ID: u64 = 0x5201;
pub const FLUENT_TESTNET_CHAIN_ID: u64 = 0x5202;
pub const FLUENT_MAINNET_CHAIN_ID: u64 = 25363;

/// Local Node (1337)
pub static FLUENT_LOCAL: LazyLock<Arc<ChainSpec>> = LazyLock::new(|| {
    let genesis = local_genesis_from_file();
    let hardforks = DEV_HARDFORKS.clone();
    ChainSpec {
        chain: Chain::from(FLUENT_LOCALNET_CHAIN_ID),
        genesis_header: SealedHeader::new_unhashed(make_genesis_header(&genesis, &hardforks)),
        genesis,
        paris_block_and_final_difficulty: Some((0, U256::from(0))),
        hardforks,
        base_fee_params: BaseFeeParamsKind::Constant(BaseFeeParams::ethereum()),
        deposit_contract: None,
        ..Default::default()
    }
    .into()
});

/// Fluent Devnet
pub static FLUENT_DEVNET: LazyLock<Arc<ChainSpec>> = LazyLock::new(|| {
    let genesis = download_and_cache_genesis_verified(&devnet_genesis())
        .expect("failed to download/verify Fluent devnet genesis");
    let hardforks = fluent_default_chain_hardforks(ForkCondition::Block(0));
    ChainSpec {
        chain: Chain::from(FLUENT_DEVNET_CHAIN_ID),
        genesis_header: SealedHeader::new_unhashed(make_genesis_header(&genesis, &hardforks)),
        genesis,
        paris_block_and_final_difficulty: Some((0, U256::from(0))),
        hardforks,
        base_fee_params: BaseFeeParamsKind::Constant(BaseFeeParams::ethereum()),
        deposit_contract: None,
        ..Default::default()
    }
    .into()
});

/// Fluent Testnet
pub static FLUENT_TESTNET: LazyLock<Arc<ChainSpec>> = LazyLock::new(|| {
    let genesis = download_and_cache_genesis_verified(&testnet_genesis())
        .expect("failed to download/verify Fluent testnet genesis");
    let hardforks = fluent_default_chain_hardforks(ForkCondition::Block(21_300_000));
    ChainSpec {
        chain: Chain::from(FLUENT_TESTNET_CHAIN_ID),
        genesis_header: SealedHeader::new_unhashed(make_genesis_header(&genesis, &hardforks)),
        genesis,
        paris_block_and_final_difficulty: Some((0, U256::from(0))),
        hardforks,
        base_fee_params: BaseFeeParamsKind::Constant(BaseFeeParams::ethereum()),
        deposit_contract: None,
        ..Default::default()
    }
    .into()
});

/// Fluent Mainnet
pub static FLUENT_MAINNET: LazyLock<Arc<ChainSpec>> = LazyLock::new(|| {
    let genesis = download_and_cache_genesis_verified(&mainnet_genesis())
        .expect("failed to download/verify Fluent mainnet genesis");
    let hardforks = fluent_default_chain_hardforks(ForkCondition::Timestamp(0));
    let genesis_header = SealedHeader::new_unhashed(make_genesis_header(&genesis, &hardforks));
    if genesis_header.timestamp != 0x69b8194c {
        panic!("malformed fluent mainnet genesis file specified: timestamp should be 0x69b8194c, make sure you're using correct genesis: {}", genesis_header.timestamp)
    }
    let genesis_hash = genesis_header.hash();
    if genesis_hash != b256!("0x7dd092d6e2aba158839db2a264d8049e7518540b342929822aac85f550c18465") {
        panic!("malformed fluent mainnet genesis file specified: genesis hash should be 0x7dd092d6e2aba158839db2a264d8049e7518540b342929822aac85f550c18465, make sure you're using correct genesis: {}", genesis_hash)
    }
    warn!("Genesis hash (Fluent Mainnet): {}", genesis_hash);
    ChainSpec {
        chain: Chain::from(FLUENT_MAINNET_CHAIN_ID),
        genesis_header,
        genesis,
        paris_block_and_final_difficulty: Some((0, U256::from(0))),
        hardforks,
        base_fee_params: BaseFeeParamsKind::Constant(BaseFeeParams::ethereum()),
        deposit_contract: None,
        ..Default::default()
    }
    .into()
});

/// Fork schedule for the protocol and the native REVM side (precompile sets, transaction rules,
/// and so on).
///
/// It does not govern EVM bytecode semantics. Contracts delegating to `PRECOMPILE_EVM_RUNTIME`
/// execute at the hardfork pinned by the deployed EVM runtime contract, which is upgraded
/// forklessly rather than activated here — so a `osaka_fork` condition scheduled in the future
/// does not hold back Osaka opcodes inside delegated bytecode. That is intended; see the
/// `fluentbase_evm::evm` module docs.
fn fluent_default_chain_hardforks(osaka_fork: ForkCondition) -> ChainHardforks {
    ChainHardforks::new(vec![
        (EthereumHardfork::Frontier.boxed(), ForkCondition::Block(0)),
        (EthereumHardfork::Homestead.boxed(), ForkCondition::Block(0)),
        (EthereumHardfork::Dao.boxed(), ForkCondition::Block(0)),
        (EthereumHardfork::Tangerine.boxed(), ForkCondition::Block(0)),
        (
            EthereumHardfork::SpuriousDragon.boxed(),
            ForkCondition::Block(0),
        ),
        (EthereumHardfork::Byzantium.boxed(), ForkCondition::Block(0)),
        (
            EthereumHardfork::Constantinople.boxed(),
            ForkCondition::Block(0),
        ),
        (
            EthereumHardfork::Petersburg.boxed(),
            ForkCondition::Block(0),
        ),
        (EthereumHardfork::Istanbul.boxed(), ForkCondition::Block(0)),
        (EthereumHardfork::Berlin.boxed(), ForkCondition::Block(0)),
        (EthereumHardfork::London.boxed(), ForkCondition::Block(0)),
        (
            EthereumHardfork::Paris.boxed(),
            ForkCondition::TTD {
                activation_block_number: 0,
                fork_block: None,
                total_difficulty: U256::ZERO,
            },
        ),
        (
            EthereumHardfork::Shanghai.boxed(),
            ForkCondition::Timestamp(0),
        ),
        (
            EthereumHardfork::Cancun.boxed(),
            ForkCondition::Timestamp(0),
        ),
        (
            EthereumHardfork::Prague.boxed(),
            ForkCondition::Timestamp(0),
        ),
        (EthereumHardfork::Osaka.boxed(), osaka_fork),
    ])
}

/// Ethereum chain specification parser.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct FluentChainSpecParser;

impl ChainSpecParser for FluentChainSpecParser {
    type ChainSpec = ChainSpec;

    const SUPPORTED_CHAINS: &'static [&'static str] =
        &["dev", "fluent-devnet", "fluent-testnet", "fluent-mainnet"];

    fn parse(s: &str) -> eyre::Result<Arc<ChainSpec>> {
        chain_value_parser(s)
    }
}

/// Clap value parser for [`ChainSpec`]s.
///
/// The value parser matches either a known chain, the path
/// to a JSON file, or a JSON-formatted string in-memory. The JSON needs to be a Genesis struct.
pub(crate) fn chain_value_parser(s: &str) -> eyre::Result<Arc<ChainSpec>, eyre::Error> {
    Ok(match s {
        "dev" => FLUENT_LOCAL.clone(),
        "fluent-devnet" => FLUENT_DEVNET.clone(),
        "fluent-testnet" => FLUENT_TESTNET.clone(),
        "fluent-mainnet" => FLUENT_MAINNET.clone(),
        _ => {
            let genesis = parse_genesis(s)?;
            // reth activates Paris from a genesis file only through `terminalTotalDifficulty`.
            // Without Paris the block environment carries no `prevrandao`, which the pre-block
            // system calls and every delegated runtime need, so a validator started from such
            // a file would fail on its first payload. Every Fluent chain starts post-merge with
            // zero difficulty, so the only meaningful value is 0: a nonzero TTD would still
            // activate Paris at block 0 through `mergeNetsplitBlock`, but describes a merge
            // that never happens. Refuse anything else up front.
            match genesis.config.terminal_total_difficulty {
                Some(ttd) if ttd.is_zero() => {}
                Some(ttd) => eyre::bail!(
                    "the genesis file sets `config.terminalTotalDifficulty` to {ttd}: every \
                     Fluent chain starts post-merge with zero difficulty, so it must be 0"
                ),
                None => eyre::bail!(
                    "the genesis file does not set `config.terminalTotalDifficulty`: every \
                     Fluent chain starts post-merge, set it to 0"
                ),
            }
            let spec: ChainSpec = genesis.into();
            if !spec.is_paris_active_at_block(0) {
                eyre::bail!("the genesis file does not activate Paris at block 0");
            }
            Arc::new(spec)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::chain_value_parser;
    use alloy_genesis::{ChainConfig, Genesis};
    use reth_chainspec::EthereumHardforks;
    use reth_revm::primitives::U256;

    /// A genesis file in the shape `crates/genesis/build.rs` writes, with the merge field under test
    fn genesis_json(terminal_total_difficulty: Option<U256>) -> String {
        let genesis = Genesis {
            config: ChainConfig {
                chain_id: 1337,
                merge_netsplit_block: Some(0),
                shanghai_time: Some(0),
                cancun_time: Some(0),
                prague_time: Some(0),
                osaka_time: Some(0),
                terminal_total_difficulty,
                terminal_total_difficulty_passed: terminal_total_difficulty.is_some(),
                ..Default::default()
            },
            ..Default::default()
        };
        serde_json::to_string(&genesis).unwrap()
    }

    #[test]
    fn genesis_file_without_terminal_total_difficulty_is_refused() {
        let err = chain_value_parser(&genesis_json(None)).unwrap_err();
        assert!(
            err.to_string().contains("terminalTotalDifficulty"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn genesis_file_with_nonzero_terminal_total_difficulty_is_refused() {
        let err = chain_value_parser(&genesis_json(Some(U256::from(1)))).unwrap_err();
        assert!(
            err.to_string().contains("terminalTotalDifficulty") && err.to_string().contains("1"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn genesis_file_with_terminal_total_difficulty_activates_paris() {
        let spec = chain_value_parser(&genesis_json(Some(U256::ZERO))).unwrap();
        assert!(spec.is_paris_active_at_block(0));
    }
}
