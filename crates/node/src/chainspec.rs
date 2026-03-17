use crate::utils::download_and_cache_genesis_verified;
use fluentbase_genesis::local_genesis_from_file;
use reth_chainspec::{
    make_genesis_header, BaseFeeParams, BaseFeeParamsKind, Chain, ChainHardforks, ChainSpec,
    EthereumHardfork, ForkCondition, Hardfork, DEV_HARDFORKS,
};
use reth_cli::chainspec::{parse_genesis, ChainSpecParser};
use reth_primitives::SealedHeader;
use reth_revm::primitives::U256;
use std::sync::{Arc, LazyLock};

/// Release tag for Fluent Mainnet genesis
const FLUENT_MAINNET_GENESIS_TAG: &str = "v1.0.0";

/// Release tag for Fluent Testnet genesis (GitHub releases).
const FLUENT_TESTNET_GENESIS_TAG: &str = "v0.3.4-dev";

/// Release tag for Fluent Devnet genesis (GitHub releases).
const FLUENT_DEVNET_GENESIS_TAG: &str = "v0.5.7";

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
    let genesis = download_and_cache_genesis_verified(FLUENT_DEVNET_GENESIS_TAG, None)
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
    let genesis = download_and_cache_genesis_verified(FLUENT_TESTNET_GENESIS_TAG, None)
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
    let genesis = download_and_cache_genesis_verified(FLUENT_MAINNET_GENESIS_TAG, Some("mainnet"))
        .expect("failed to download/verify Fluent mainnet genesis");
    let hardforks = fluent_default_chain_hardforks(ForkCondition::Timestamp(0));
    ChainSpec {
        chain: Chain::from(FLUENT_MAINNET_CHAIN_ID),
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
        _ => Arc::new(parse_genesis(s)?.into()),
    })
}
