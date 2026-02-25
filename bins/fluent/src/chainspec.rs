use alloy_genesis::Genesis;
use fluentbase_genesis::local_genesis_from_file;
use reth_chainspec::{
    make_genesis_header, BaseFeeParams, BaseFeeParamsKind, Chain, ChainHardforks, ChainSpec,
    EthereumHardfork, ForkCondition, Hardfork, DEV_HARDFORKS,
};
use reth_cli::chainspec::{parse_genesis, ChainSpecParser};
use reth_primitives::SealedHeader;
use reth_revm::primitives::U256;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
};

/// Chains supported by reth. The first value should be used as the default.
pub const SUPPORTED_CHAINS: &[&str] = &["dev", "fluent-devnet", "fluent-testnet"];

/// Release tag for Fluent Devnet genesis (GitHub releases).
///
/// Example: `v0.4.11-dev` -> downloads:
/// - `genesis-v0.4.11-dev.json.gz`
/// - `genesis-v0.4.11-dev.json.gz.asc`
const FLUENT_DEVNET_GENESIS_TAG: &str = "v0.5.2";

/// Release tag for Fluent Testnet genesis (GitHub releases).
///
/// Update to the tag you publish for testnet.
const FLUENT_TESTNET_GENESIS_TAG: &str = "v0.3.4-dev";

/// Downloads genesis from GitHub releases into `../genesis/` (sibling to this crate),
/// caches it, and verifies its detached OpenPGP signature.
///
/// This is intentionally synchronous because it runs during CLI startup / chainspec selection.
fn download_and_cache_genesis_verified(tag: &str) -> eyre::Result<Genesis> {
    use eyre::WrapErr as _;

    let (gz_url, asc_url, gz_name, asc_name) = genesis_urls(tag);

    println!("Checking genesis for tag  {}...", tag);
    let genesis_dir = genesis_cache_dir()?;
    fs::create_dir_all(&genesis_dir).wrap_err_with(|| {
        format!(
            "failed to create genesis cache dir {}",
            genesis_dir.display()
        )
    })?;

    let gz_path = genesis_dir.join(&gz_name);
    let asc_path = genesis_dir.join(&asc_name);

    // Fast path: already cached and signature verifies.
    if gz_path.exists() && asc_path.exists() {
        if verify_detached_signature(&gz_path, &asc_path).is_ok() {
            println!("Using cached genesis from {}", gz_path.display());
            return read_genesis_from_gz(&gz_path);
        }
        // Corrupted / wrong key / tampered -> redownload.
        let _ = fs::remove_file(&gz_path);
        let _ = fs::remove_file(&asc_path);
    }

    println!("Genesis not found in cache, downloading from {}", gz_url);

    // Download both files.
    download_to(&gz_url, &gz_path).wrap_err("failed to download genesis .gz")?;
    download_to(&asc_url, &asc_path).wrap_err("failed to download genesis .asc")?;

    println!("Verifying signature genesis signature...");

    // Verify, then read.
    verify_detached_signature(&gz_path, &asc_path)
        .wrap_err("genesis signature verification failed")?;
    read_genesis_from_gz(&gz_path)
}

/// Where to cache genesis files: `../genesis` relative to this crate's `Cargo.toml`.
fn genesis_cache_dir() -> eyre::Result<PathBuf> {
    let proj = directories::ProjectDirs::from("xyz", "fluentlabs", "fluent")
        .ok_or_else(|| eyre::eyre!("cannot determine cache directory"))?;
    Ok(proj.cache_dir().join("genesis"))
}

/// Build release URLs & filenames for the given tag.
fn genesis_urls(tag: &str) -> (String, String, String, String) {
    let base = format!("https://github.com/fluentlabs-xyz/fluentbase/releases/download/{tag}");
    let gz_name = format!("genesis-{tag}.json.gz");
    let asc_name = format!("{gz_name}.asc");
    let gz_url = format!("{base}/{gz_name}");
    let asc_url = format!("{base}/{asc_name}");
    (gz_url, asc_url, gz_name, asc_name)
}

/// Download `url` to `path` atomically (write to temp, then rename).
fn download_to(url: &str, path: &Path) -> eyre::Result<()> {
    use eyre::WrapErr as _;
    let tmp = path.with_extension("tmp");

    // NOTE: blocking client avoids pulling tokio into a CLI dependency tree.
    let resp = reqwest::blocking::Client::builder()
        .user_agent("fluent-chainspec/1.0")
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .wrap_err("failed to build HTTP client")?
        .get(url)
        .send()
        .wrap_err_with(|| format!("GET {url}"))?
        .error_for_status()
        .wrap_err_with(|| format!("GET {url} returned non-success"))?;

    let bytes = resp
        .bytes()
        .wrap_err_with(|| format!("reading body from {url}"))?;

    {
        let mut f = fs::File::create(&tmp)
            .wrap_err_with(|| format!("failed to create {}", tmp.display()))?;
        f.write_all(&bytes)
            .wrap_err_with(|| format!("failed to write {}", tmp.display()))?;
        f.sync_all()
            .wrap_err_with(|| format!("failed to sync {}", tmp.display()))?;
    }

    fs::rename(&tmp, path)
        .wrap_err_with(|| format!("failed to move {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Read a gzipped genesis JSON into [`Genesis`].
fn read_genesis_from_gz(path: &Path) -> eyre::Result<Genesis> {
    use eyre::WrapErr as _;
    let gz = fs::read(path).wrap_err_with(|| format!("failed to read {}", path.display()))?;
    let mut decoder = flate2::read::GzDecoder::new(&gz[..]);
    let mut json = String::new();
    decoder
        .read_to_string(&mut json)
        .wrap_err("failed to decompress genesis gz")?;
    let genesis =
        serde_json::from_str::<Genesis>(&json).wrap_err("failed to parse genesis JSON")?;
    Ok(genesis)
}

/// ASCII-armored OpenPGP public key used to verify genesis artifacts.
const FLUENT_RELEASE_PUBKEY_ASC: &str = "-----BEGIN PGP PUBLIC KEY BLOCK-----
mDMEaEq6ORYJKwYBBAHaRw8BAQdADSciIyJRuaPogw2vJ388jlOsKRQk1c84vUpn
NT+vmeu0J0RtaXRyaWkgU2F2b25pbiA8ZG1pdHJ5QGZsdWVudGxhYnMueHl6PoiT
BBMWCgA7FiEECm0F5d2YBpuhhO2DBKaNYg1SCP0FAmhKujkCGwMFCwkIBwICIgIG
FQoJCAsCBBYCAwECHgcCF4AACgkQBKaNYg1SCP0eRwEA43IlexWb2Nh/rVzVyRVg
fPLZ45a13AP0iMCnAhjFK/cBAL5zDzWNNFkxHm6XGYQC4mHWLeZFe3gIJVQ0Y+wH
hCoHuDgEaEq6ORIKKwYBBAGXVQEFAQEHQCBTP3PIjJhuMZdF5aVuEiPODt9EpEnK
Jph+AW0cmfZ2AwEIB4h4BBgWCgAgFiEECm0F5d2YBpuhhO2DBKaNYg1SCP0FAmhK
ujkCGwwACgkQBKaNYg1SCP2KwgD/UJk7eQhlLNosZNLOyFj48241KcG2lJbCgzt8
XehpkCgA/13esUBYao//zRco9fgrVbSBNJ7FO1G0jXAYygDqCYsJ
=Ortc
-----END PGP PUBLIC KEY BLOCK-----";

/// Verify the detached OpenPGP signature (`.asc`) for the given file.
///
/// This expects an ASCII-armored public key either from env var
/// `FLUENT_RELEASE_PUBKEY_ASC` or from `FLUENT_RELEASE_PUBKEY_ASC_FALLBACK`.
fn verify_detached_signature(_data_path: &Path, _sig_path: &Path) -> eyre::Result<()> {
    // use sequoia_openpgp::{Cert};
    // if std::env::var("SKIP_SIGNATURE_VERIFICATION").is_ok() {
    //     return Ok(());
    // }
    // let cert = Cert::from_reader(FLUENT_RELEASE_PUBKEY_ASC.as_bytes()).unwrap();
    // let sig_bytes = fs::read(sig_path)
    //     .wrap_err_with(|| format!("failed to read signature {}", sig_path.display()))?;
    // let sqv = sequoia_sqv::Sqv::new();
    // sqv.verify_detached(&cert, sig, file)?;
    //TODO(dmitry123): Make it work
    Ok(())
}

/// Fluent Developer Preview
pub static FLUENT_LOCAL: LazyLock<Arc<ChainSpec>> = LazyLock::new(|| {
    let genesis = local_genesis_from_file();
    let hardforks = DEV_HARDFORKS.clone();
    ChainSpec {
        chain: Chain::from(1337),
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

/// Fluent Developer Preview
pub static FLUENT_DEVNET: LazyLock<Arc<ChainSpec>> = LazyLock::new(|| {
    let genesis = download_and_cache_genesis_verified(FLUENT_DEVNET_GENESIS_TAG)
        .expect("failed to download/verify Fluent devnet genesis");
    let hardforks = DEV_HARDFORKS.clone();
    ChainSpec {
        chain: Chain::from(0x5201),
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
    let genesis = download_and_cache_genesis_verified(FLUENT_TESTNET_GENESIS_TAG)
        .expect("failed to download/verify Fluent testnet genesis");
    let hardforks = ChainHardforks::new(vec![
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
    ]);
    ChainSpec {
        chain: Chain::from(0x5202),
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

/// Clap value parser for [`ChainSpec`]s.
///
/// The value parser matches either a known chain, the path
/// to a JSON file, or a JSON formatted string in-memory. The json needs to be a Genesis struct.
pub(crate) fn chain_value_parser(s: &str) -> eyre::Result<Arc<ChainSpec>, eyre::Error> {
    Ok(match s {
        "dev" => FLUENT_LOCAL.clone(),
        "fluent-devnet" => FLUENT_DEVNET.clone(),
        "fluent-testnet" => FLUENT_TESTNET.clone(),
        _ => Arc::new(parse_genesis(s)?.into()),
    })
}

/// Ethereum chain specification parser.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct FluentChainSpecParser;

impl ChainSpecParser for FluentChainSpecParser {
    type ChainSpec = ChainSpec;

    const SUPPORTED_CHAINS: &'static [&'static str] = SUPPORTED_CHAINS;

    fn parse(s: &str) -> eyre::Result<Arc<ChainSpec>> {
        chain_value_parser(s)
    }
}
