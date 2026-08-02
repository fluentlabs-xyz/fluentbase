use alloy_genesis::Genesis;
use clap::ValueEnum;
use commonware_codec::Encode;
use commonware_cryptography::Signer as _;
use eyre::WrapErr;
use serde::Serialize;
use std::fs;
use std::net::IpAddr;
use std::path::Path;

use crate::bootstrap::{CHAIN_CONFIG_ADDR, STAKING_ADDR};
use crate::keys::KeySet;

/// How `peers.json` renders each validator's ingress host.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum PeerHostMode {
    /// Pinned docker-network IP literal (`172.20.0.N:9000`) — the default,
    /// byte-identical to the pre-DNS behaviour.
    #[default]
    Ip,
    /// Docker service-name hostname (`validator-N:9000`). Requires the
    /// validator network's `ALLOW_DNS` policy to be `true` (all nodes upgraded
    /// together).
    Dns,
}

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

// Devnet generator: a flat positional signature keeps the single call site
// readable; the DNS peer-host mode + seed-DNS-zone add the 8th/9th parameters.
#[allow(clippy::too_many_arguments)]
pub fn write(
    out: &Path,
    genesis: &Genesis,
    keys: &KeySet,
    bootstrap_count: usize,
    validator_ips: &[IpAddr],
    peer_host_mode: PeerHostMode,
    seed_dns_zone: Option<&str>,
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
        // Merge-preserving (see write_shared_json_preserving): staking-reader.json is a
        // fixed object with no per-idx array, so this is a plain write — the helper is
        // applied uniformly to all three shared files and no-ops on a non-array target.
        write_shared_json_preserving(out, "staking-reader.json", &serde_json::to_value(&sr)?, "")?;
    }

    // peers.json `socket` is a `"host:port"` string parsed by
    // `fluentbase_p2p::parse_ingress` (both `BootstrapperJson.socket` and
    // `--dpos.dialable`). Default `Ip` mode emits the pinned docker-network IP
    // literal (`172.20.0.N:9000` → `Ingress::Socket`), byte-identical to the
    // pre-DNS behaviour. `Dns` mode emits the docker service-name hostname
    // (`validator-N:9000` → `Ingress::Dns`), which requires the validator
    // network's `ALLOW_DNS` policy (constants::ALLOW_DNS) to be `true`.
    let peers: Vec<PeerEntry> = keys
        .validators
        .iter()
        .take(bootstrap_count)
        .map(|v| {
            let pk = v.peer.public_key();
            let pk_bytes = pk.encode();
            let socket = match peer_host_mode {
                PeerHostMode::Ip => format!("{}:9000", validator_ips[v.idx as usize]),
                PeerHostMode::Dns => format!("validator-{}:9000", v.idx),
            };
            PeerEntry {
                peer_pubkey: format!("0x{}", hex::encode(pk_bytes.as_ref())),
                socket,
            }
        })
        .collect();
    // Merge-preserving: the document root IS the array, so any surplus tail a runtime
    // rewrite grew is kept (never shrink an existing peers list).
    write_shared_json_preserving(out, "peers.json", &serde_json::to_value(&peers)?, "")?;

    // DNS-bootstrap verification path: emit a CoreDNS zone + Corefile serving the
    // first `bootstrap_count` validators as `pubkey@validator-N:9000` TXT records
    // on `seed_dns_zone`, so `--dpos.bootstrappers=dns:<zone>` resolves. Non-seed
    // queries forward to docker's embedded resolver (127.0.0.11) so service-name
    // lookups still work. Only emitted in `--seed-dns-zone` mode; peers.json above
    // is written regardless (harmless in DNS mode).
    if let Some(zone) = seed_dns_zone {
        write_seed_dns(out, zone, keys, bootstrap_count)?;
    }

    // Reth p2p enode URL for validator-0 — followers pin this as
    // `--trusted-peers=$(cat v0-enode.txt)` so eth/68 P2P sync can backfill
    // historical sequencer-era blocks the sequencer-url WS feed never re-emits.
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
    // Merge-preserving: the owner-address array lives under "/validators". A stray
    // pool-scoped genesis re-run must NOT revert this file to the pool prefix and drop
    // minted idx >= POOL that a runtime `write-keys --idx N` had appended.
    write_shared_json_preserving(
        out,
        "addresses.json",
        &serde_json::to_value(&addresses)?,
        "/validators",
    )?;

    Ok(())
}

/// Emit a CoreDNS `Corefile` + `<zone>.zone` into `out` serving the first
/// `bootstrap_count` validators as `pubkey@validator-N:9000` TXT records on
/// `zone`. The `.` server forwards everything else to docker's embedded DNS
/// (127.0.0.11) so service-name resolution keeps working for containers pointed
/// at this CoreDNS. Consumed by `--dpos.bootstrappers=dns:<zone>`.
fn write_seed_dns(
    out: &Path,
    zone: &str,
    keys: &KeySet,
    bootstrap_count: usize,
) -> eyre::Result<()> {
    let mut records = String::new();
    for v in keys.validators.iter().take(bootstrap_count) {
        let pk_hex = hex::encode(v.peer.public_key().encode().as_ref());
        records.push_str(&format!(
            "@ IN TXT \"0x{pk_hex}@validator-{}:9000\"\n",
            v.idx
        ));
    }
    // RFC-1035 master file. Single SOA/NS (self-hosted `ns`), 60 s TTLs (devnet).
    let zonefile = format!(
        "$ORIGIN {zone}.\n\
         $TTL 60\n\
         @ IN SOA ns.{zone}. admin.{zone}. ( 1 60 60 60 60 )\n\
         @ IN NS ns.{zone}.\n\
         ns IN A 127.0.0.1\n\
         {records}"
    );
    fs::write(out.join(format!("{zone}.zone")), zonefile)?;

    let corefile = format!(
        "{zone}:53 {{\n\
        \x20   file /runtime/{zone}.zone\n\
        \x20   errors\n\
         }}\n\
         .:53 {{\n\
        \x20   forward . 127.0.0.11\n\
        \x20   errors\n\
         }}\n"
    );
    fs::write(out.join("Corefile"), corefile)?;
    Ok(())
}

/// Serialize `doc` to `out/file`, but if that file already holds a JSON document whose
/// array at `array_pointer` (a serde_json pointer; "" = the document root IS the array)
/// is LONGER than `doc`'s, append the existing surplus TAIL (indices >= the new length)
/// so a pool-scoped genesis re-run never SHRINKS a shared file a runtime `write-keys
/// --idx N` had grown. Minted validator identities (idx >= POOL) live ONLY in these
/// shared JSONs until their durable per-idx key files are read, so a wholesale rewrite
/// that reverted a file to the pool prefix would silently drop them (geo-soak v53 r572:
/// validator-20 minted at r456, addresses.json back to 20 entries by r572, promote died
/// on a null owner addr). Never shrinks an array. A non-array target (staking-reader.json
/// is a fixed object) has no tail to preserve — it is written as-is. An unparseable
/// existing file is overwritten (a corrupt file must not brick genesis-init) after a
/// stderr warning.
fn write_shared_json_preserving(
    out: &Path,
    file: &str,
    doc: &serde_json::Value,
    array_pointer: &str,
) -> eyre::Result<()> {
    let path = out.join(file);
    let mut corrupt = false;
    let merged = merge_surplus_tail(
        doc.clone(),
        fs::read(&path).ok().as_deref(),
        array_pointer,
        &mut corrupt,
    );
    if corrupt {
        eprintln!(
            "genesis-bootstrap: WARNING: existing {} is unparseable — overwriting \
             (any minted-idx tail it held is NOT preserved)",
            path.display()
        );
    }
    fs::write(&path, serde_json::to_string_pretty(&merged)?)?;
    Ok(())
}

/// Pure core of [`write_shared_json_preserving`] (see its doc): given the new document
/// and the RAW bytes of the existing on-disk file (`None` if absent), return the document
/// to write with any existing surplus tail preserved. Sets `*corrupt` iff the existing
/// bytes are present but not valid JSON.
fn merge_surplus_tail(
    mut doc: serde_json::Value,
    existing: Option<&[u8]>,
    array_pointer: &str,
    corrupt: &mut bool,
) -> serde_json::Value {
    let Some(bytes) = existing else { return doc };
    let old = match serde_json::from_slice::<serde_json::Value>(bytes) {
        Ok(v) => v,
        Err(_) => {
            *corrupt = true;
            return doc;
        }
    };
    let old_arr = match old.pointer(array_pointer).and_then(|v| v.as_array()) {
        Some(a) => a.clone(),
        None => return doc, // existing file is not an array at this pointer — nothing to preserve
    };
    if let Some(new_arr) = doc
        .pointer_mut(array_pointer)
        .and_then(|v| v.as_array_mut())
    {
        if old_arr.len() > new_arr.len() {
            new_arr.extend_from_slice(&old_arr[new_arr.len()..]);
        }
    }
    doc
}

/// Materialize ONLY validator `idx`'s PRIVATE key files (bls/peer/slasher keystore +
/// password + owner key) onto `out`, and (re)write `addresses.json` covering
/// `validators[0..=idx]`. Reuses the EXACT private writers `write` runs in its
/// genesis loop, so idx<POOL files come out byte-identical to genesis (the keystore
/// RNG is `OsRng` so `slasher.json` ciphertext differs — irrelevant for a fresh idx
/// never baked, and `slasher.pwd` is deterministic so it always decrypts). Used by
/// the `write-keys` subcommand to mint a FRESH idx (>= POOL) at runtime for the soak's
/// mint-on-demand — it does NOT touch the load-bearing genesis JSONs
/// (`genesis-local.json` / `staking-reader.json` / `peers.json` / `v0-enode.txt`), so
/// the running chain is untouched (`addresses.json` is a pure host-side lookup —
/// deterministic, live nodes never read it).
pub fn write_validator_keys(out: &Path, keys: &KeySet, idx: u32) -> eyre::Result<()> {
    let v = keys.validators.get(idx as usize).ok_or_else(|| {
        eyre::eyre!(
            "write_validator_keys: idx {idx} out of range for derived pool of {}",
            keys.validators.len()
        )
    })?;

    let keys_dir = out.join("keys");
    let dir = keys_dir.join(format!("validator-{}", v.idx));
    fs::create_dir_all(&dir)?;

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

    // owner-N.hex — the soak signs this idx's register/setkeys/stake with `pp_owner_key N`.
    write_mode_0600(
        &keys_dir.join(format!("owner-{}.hex", v.idx)),
        hex::encode(v.l2_signer.to_bytes()).as_bytes(),
    )?;

    // (re)write addresses.json covering validators[0..=idx] — `pp_owner_addr N` reads
    // addresses.json[N]; without it every register/setkeys/stake call-site breaks for
    // idx>=POOL. Deterministic + byte-identical for 0..POOL-1 → safe to rewrite.
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

#[cfg(test)]
mod tests {
    use super::merge_surplus_tail;
    use serde_json::json;

    #[test]
    fn preserves_longer_existing_tail() {
        // addresses.json shape: array under "/validators". A pool-scoped re-run (2 entries)
        // over a grown file (4 entries) must keep the minted tail [c, d].
        let new = json!({ "validators": ["a", "b"] });
        let existing = serde_json::to_vec(&json!({ "validators": ["a", "b", "c", "d"] })).unwrap();
        let mut corrupt = false;
        let merged = merge_surplus_tail(new, Some(&existing), "/validators", &mut corrupt);
        assert!(!corrupt);
        assert_eq!(merged["validators"], json!(["a", "b", "c", "d"]));
    }

    #[test]
    fn shorter_or_equal_existing_keeps_new() {
        let new = json!({ "validators": ["a", "b", "c"] });
        let existing = serde_json::to_vec(&json!({ "validators": ["a", "b"] })).unwrap();
        let mut corrupt = false;
        let merged = merge_surplus_tail(new, Some(&existing), "/validators", &mut corrupt);
        assert!(!corrupt);
        assert_eq!(merged["validators"], json!(["a", "b", "c"]));
    }

    #[test]
    fn absent_existing_leaves_new_unchanged() {
        let new = json!(["x", "y"]);
        let mut corrupt = false;
        let merged = merge_surplus_tail(new.clone(), None, "", &mut corrupt);
        assert!(!corrupt);
        assert_eq!(merged, new);
    }

    #[test]
    fn root_array_pointer_preserves_tail() {
        // peers.json shape: the document root IS the array.
        let new = json!([{ "peer": 0 }]);
        let existing =
            serde_json::to_vec(&json!([{ "peer": 0 }, { "peer": 1 }, { "peer": 2 }])).unwrap();
        let mut corrupt = false;
        let merged = merge_surplus_tail(new, Some(&existing), "", &mut corrupt);
        assert!(!corrupt);
        assert_eq!(merged.as_array().unwrap().len(), 3);
    }

    #[test]
    fn unparseable_existing_flags_corrupt_and_overwrites() {
        let new = json!({ "validators": ["a"] });
        let mut corrupt = false;
        let merged = merge_surplus_tail(
            new.clone(),
            Some(b"{ not json"),
            "/validators",
            &mut corrupt,
        );
        assert!(corrupt);
        assert_eq!(merged, new);
    }

    #[test]
    fn non_array_target_written_as_is() {
        // staking-reader.json shape: a fixed object, no tail to preserve.
        let new = json!({ "staking_address": "0xnew" });
        let existing = serde_json::to_vec(&json!({ "staking_address": "0xold" })).unwrap();
        let mut corrupt = false;
        let merged = merge_surplus_tail(new.clone(), Some(&existing), "", &mut corrupt);
        assert!(!corrupt);
        assert_eq!(merged, new);
    }
}
