//! Bootstrappers loader: parse operator-supplied JSON list of
//! `(Ed25519 peer pubkey, SocketAddr)` pairs for cold-start peer discovery.
//!
//! No in-tree per-chain default lists — operator MUST provide a JSON file
//! via `--dpos.bootstrappers` (genesis bootstrap event = empty `[]`
//! JSON file). This avoids:
//! - Silent prod deployment with empty defaults (no defense before).
//! - Chain-ID duplication between `bootstrappers.rs` and `chainspec.rs`.
//! - In-tree placeholder lists that drift from foundation's deployed bootnodes.
//!
//! Format normalization contract: see [`load_from_json_path`].

use std::net::SocketAddr;

use commonware_codec::DecodeExt as _;
use commonware_p2p::{authenticated::discovery::Bootstrapper, Ingress};
use fluentbase_bls::PeerPubkey;

/// JSON schema for [`load_from_json_path`] (de-by-serde).
#[derive(serde::Deserialize)]
struct BootstrapperJson {
    /// Hex-encoded Ed25519 peer pubkey (32 bytes); accepts `0x`-prefixed or bare.
    peer_pubkey: String,
    /// Socket address (e.g. `"10.0.0.1:9000"`).
    socket: SocketAddr,
}

/// Load bootstrappers from a JSON file. Format:
///
/// ```json
/// [
///   {"peer_pubkey": "0x...32bytes", "socket": "10.0.0.1:9000"},
///   ...
/// ]
/// ```
///
/// Each `peer_pubkey` is subgroup-checked at decode (`PeerPubkey::decode`).
///
/// Operator MUST provide a file via `--dpos.bootstrappers`; genesis
/// bootstrap event = empty `[]` JSON file (explicit intent for the first
/// bootnode in a new network).
///
/// **Format normalization contract** (pinned to avoid platform/version drift):
/// - `peer_pubkey`: lowercase hex of 32 raw bytes; `0x` prefix is **optional**
///   (accepted via `commonware_utils::from_hex_formatted`). Trailing whitespace
///   is trimmed.
/// - `socket`: standard Rust `std::net::SocketAddr` literal. IPv6 addresses
///   **require brackets** (e.g. `"[::1]:9000"`); bare `"::1:9000"` will fail
///   to parse on all platforms. IPv4 is `"host:port"` as expected.
pub fn load_from_json_path<P: AsRef<std::path::Path>>(
    path: P,
) -> eyre::Result<Vec<Bootstrapper<PeerPubkey>>> {
    let raw = std::fs::read_to_string(path.as_ref())
        .map_err(|e| eyre::eyre!("failed reading bootstrappers JSON: {e}"))?;
    let entries: Vec<BootstrapperJson> =
        serde_json::from_str(&raw).map_err(|e| eyre::eyre!("malformed bootstrappers JSON: {e}"))?;
    entries
        .into_iter()
        .map(|e| {
            let bytes = commonware_utils::from_hex_formatted(e.peer_pubkey.trim())
                .ok_or_else(|| eyre::eyre!("bootstrapper peer_pubkey not valid hex"))?;
            let pk = PeerPubkey::decode(bytes.as_slice())
                .map_err(|e| eyre::eyre!("bootstrapper peer_pubkey decode failed: {e:?}"))?;
            Ok((pk, Ingress::Socket(e.socket)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_from_json_path_round_trips() {
        use commonware_codec::Encode as _;
        use commonware_cryptography::{ed25519::PrivateKey, Signer};
        use commonware_math::algebra::Random as _;
        use rand_core::SeedableRng as _;
        let mut rng = rand_08::rngs::StdRng::seed_from_u64(42);
        let sk_a = PrivateKey::random(&mut rng);
        let sk_b = PrivateKey::random(&mut rng);
        let pk_a_bytes = sk_a.public_key().encode();
        let pk_b_bytes = sk_b.public_key().encode();
        let hex_a = hex::encode(pk_a_bytes.as_ref());
        let hex_b = hex::encode(pk_b_bytes.as_ref());
        let json = format!(
            r#"[{{"peer_pubkey":"0x{hex_a}","socket":"10.0.0.1:9000"}},{{"peer_pubkey":"{hex_b}","socket":"10.0.0.2:9001"}}]"#,
        );
        let dir = std::env::temp_dir();
        let path = dir.join(format!("bootstrappers_test_rt_{}.json", std::process::id()));
        std::fs::write(&path, json).unwrap();

        let loaded = load_from_json_path(&path).expect("round trip");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].0.encode().as_ref(), pk_a_bytes.as_ref());
        assert_eq!(loaded[1].0.encode().as_ref(), pk_b_bytes.as_ref());
        match (&loaded[0].1, &loaded[1].1) {
            (Ingress::Socket(a), Ingress::Socket(b)) => {
                assert_eq!(*a, "10.0.0.1:9000".parse::<SocketAddr>().unwrap());
                assert_eq!(*b, "10.0.0.2:9001".parse::<SocketAddr>().unwrap());
            }
            _ => panic!("unexpected Ingress variant"),
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn load_from_json_path_rejects_invalid_hex() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "bootstrappers_test_invhex_{}.json",
            std::process::id()
        ));
        std::fs::write(
            &path,
            r#"[{"peer_pubkey":"zzzzzz","socket":"127.0.0.1:9000"}]"#,
        )
        .unwrap();
        assert!(load_from_json_path(&path).is_err());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn load_from_json_path_rejects_short_pubkey() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "bootstrappers_test_short_{}.json",
            std::process::id()
        ));
        std::fs::write(
            &path,
            r#"[{"peer_pubkey":"0xdeadbeef","socket":"127.0.0.1:9000"}]"#,
        )
        .unwrap();
        assert!(load_from_json_path(&path).is_err());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn load_from_json_path_rejects_missing_file() {
        assert!(load_from_json_path("/this/path/does/not/exist.json").is_err());
    }
}
