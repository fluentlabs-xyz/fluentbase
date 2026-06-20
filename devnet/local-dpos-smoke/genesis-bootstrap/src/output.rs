use alloy_genesis::Genesis;
use commonware_codec::Encode;
use commonware_cryptography::Signer as _;
use eyre::WrapErr;
use serde::Serialize;
use std::fs;
use std::net::IpAddr;
use std::path::Path;

use crate::bootstrap::{CHAIN_CONFIG_ADDR, STAKING_ADDR};
use crate::keys::KeySet;

#[derive(Serialize)]
struct StakingReaderJson {
    staking_address: String,
    chain_config_address: String,
    /// Runtime-deployed liveness is NOT at the canonical predeploy slot, so
    /// the bare/pre-written variant pins it explicitly; the genesis-baked
    /// variant keeps relying on the reader's serde default.
    #[serde(skip_serializing_if = "Option::is_none")]
    liveness_slashing_address: Option<String>,
}

#[derive(Serialize)]
struct PeerEntry {
    peer_pubkey: String,
    socket: String,
}

#[derive(Serialize)]
struct AddressesJson {
    validators: Vec<String>,
}

pub fn write(
    out: &Path,
    genesis: &Genesis,
    keys: &KeySet,
    bootstrap_count: usize,
    validator_ips: &[IpAddr],
    bare: bool,
    staking_reader_create_nonces: Option<&[u64]>,
) -> eyre::Result<()> {
    fs::write(
        out.join("genesis-local.json"),
        serde_json::to_string_pretty(genesis)?,
    )?;

    // `Full` flow: the cluster lives at fixed genesis predeploy slots. Bare
    // flow: the cluster is forge-deployed at runtime by the driver, but the
    // production-path smoke pre-writes the reader config by predicting the
    // CREATE proxy addresses from the deployer (owner-0) nonces, so every
    // node can carry `--dpos.staking-config` from first boot; the driver
    // asserts the deploy manifest equals this file.
    let staking_reader = if bare {
        staking_reader_create_nonces.map(|nonces| {
            let deployer = keys.validators[0].l2_signer.address();
            StakingReaderJson {
                staking_address: format!("{:#x}", deployer.create(nonces[0])),
                chain_config_address: format!("{:#x}", deployer.create(nonces[1])),
                liveness_slashing_address: Some(format!("{:#x}", deployer.create(nonces[2]))),
            }
        })
    } else {
        Some(StakingReaderJson {
            staking_address: format!("{:#x}", STAKING_ADDR),
            chain_config_address: format!("{:#x}", CHAIN_CONFIG_ADDR),
            liveness_slashing_address: None,
        })
    };
    if let Some(sr) = staking_reader {
        fs::write(
            out.join("staking-reader.json"),
            serde_json::to_string_pretty(&sr)?,
        )?;
    }

    // peers.json socket uses pinned IP, NOT docker service-name hostname,
    // because both `BootstrapperJson.socket` (crates/p2p/src/bootstrappers.rs:25)
    // and `--dpos.dialable` (bins/fluent/src/main.rs:165) deserialise as
    // `SocketAddr`, and `std::net::SocketAddr::from_str` rejects any
    // non-IP host literal.
    let peers: Vec<PeerEntry> = keys
        .validators
        .iter()
        .take(bootstrap_count)
        .map(|v| {
            let pk = v.peer.public_key();
            let pk_bytes = pk.encode();
            PeerEntry {
                peer_pubkey: format!("0x{}", hex::encode(pk_bytes.as_ref())),
                socket: format!("{}:9000", validator_ips[v.idx as usize]),
            }
        })
        .collect();
    fs::write(
        out.join("peers.json"),
        serde_json::to_string_pretty(&peers)?,
    )?;

    // Reth p2p enode URL for validator-0 — followers pin this as
    // `--trusted-peers=$(cat v0-enode.txt)` so eth/68 P2P sync can backfill
    // historical Tempo blocks the sequencer-url WS feed never re-emits.
    {
        let secp = secp256k1::Secp256k1::new();
        let v0 = &keys.validators[0];
        let pk = secp256k1::PublicKey::from_secret_key(&secp, &v0.reth_p2p);
        let uncompressed = pk.serialize_uncompressed();
        // enode pubkey = uncompressed point WITHOUT the 0x04 prefix (64 bytes / 128 hex chars).
        let pubkey_hex = hex::encode(&uncompressed[1..]);
        let enode = format!("enode://{}@{}:30303", pubkey_hex, validator_ips[0]);
        fs::write(out.join("v0-enode.txt"), enode.as_bytes())?;
        let secret_hex = hex::encode(v0.reth_p2p.secret_bytes());
        write_mode_0600(&out.join("v0-p2p-secret.hex"), secret_hex.as_bytes())?;
    }

    let keys_dir = out.join("keys");
    fs::create_dir_all(&keys_dir)?;
    for v in &keys.validators {
        let dir = keys_dir.join(format!("validator-{}", v.idx));
        fs::create_dir_all(&dir)?;

        // BLS plaintext hex via the in-crate writer added in Phase 1
        // (`secret()` is `pub(crate)`, cannot reach across
        // crate boundary).
        v.bls
            .write_to_plaintext_file(dir.join("bls.hex"))
            .map_err(|e| eyre::eyre!("write BLS plaintext: {e:?}"))?;

        let peer_bytes = v.peer.encode();
        write_mode_0600(
            &dir.join("peer.hex"),
            hex::encode(peer_bytes.as_ref()).as_bytes(),
        )?;

        let mut rng = rand_08::rngs::OsRng;
        let pk_bytes: alloy_primitives::B256 = v.slasher.to_bytes();
        let (_loaded, _file_name) = alloy_signer_local::LocalSigner::encrypt_keystore(
            &dir,
            &mut rng,
            pk_bytes,
            v.slasher_password.as_bytes(),
            Some("slasher.json"),
        )
        .wrap_err("encrypt slasher keystore")?;
        write_mode_0600(&dir.join("slasher.pwd"), v.slasher_password.as_bytes())?;

        // No genesis beacon share is written: the beacon is always-on live DKG, so
        // each validator computes its own (PK_E, share) from the epoch-2 ceremony and
        // persists it to `<datadir>/beacon/` at runtime — there is no genesis-baked
        // share/sharing/PK material on any stack.
    }

    let full_dir = keys_dir.join("full-node");
    fs::create_dir_all(&full_dir)?;
    let fn_peer_bytes = keys.full_node.peer.encode();
    write_mode_0600(
        &full_dir.join("peer.hex"),
        hex::encode(fn_peer_bytes.as_ref()).as_bytes(),
    )?;

    // DEVNET ONLY — funded dev key (validator-0 l2_signer, already 1 ETH at genesis)
    // so the smoke regression cases can sign txs via `cast send --private-key 0x$(cat
    // funded.hex)`. Bare hex (no 0x prefix). Plus the validator index→address map for
    // per-validator on-chain assertions (byzantine tombstone, liveness) without
    // recomputing the custom key derivation on the host.
    write_mode_0600(
        &keys_dir.join("funded.hex"),
        hex::encode(keys.validators[0].l2_signer.to_bytes()).as_bytes(),
    )?;
    // Governance signer key — owns ChainConfig/Staking; smoke-gov-interval uses it to
    // call setEpochBlockInterval. DEVNET ONLY (1 ETH at genesis).
    write_mode_0600(
        &keys_dir.join("governance.hex"),
        hex::encode(keys.governance_signer.to_bytes()).as_bytes(),
    )?;
    // Every validator's l2 owner key (bare hex, no 0x). The production-path
    // driver signs per-validator register/vote/delegate txs with these; the
    // genesis-baked smoke only ever needed validator-0 (`funded.hex`).
    for v in &keys.validators {
        write_mode_0600(
            &keys_dir.join(format!("owner-{}.hex", v.idx)),
            hex::encode(v.l2_signer.to_bytes()).as_bytes(),
        )?;
    }
    let addresses = AddressesJson {
        validators: keys
            .validators
            .iter()
            .map(|v| format!("{:#x}", v.l2_signer.address()))
            .collect(),
    };
    fs::write(
        out.join("addresses.json"),
        serde_json::to_string_pretty(&addresses)?,
    )?;

    Ok(())
}

#[cfg(unix)]
fn write_mode_0600(path: &Path, data: &[u8]) -> eyre::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(data)?;
    Ok(())
}

#[cfg(not(unix))]
fn write_mode_0600(path: &Path, data: &[u8]) -> eyre::Result<()> {
    fs::write(path, data)?;
    Ok(())
}
