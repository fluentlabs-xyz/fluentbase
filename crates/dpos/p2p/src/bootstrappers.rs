//! Bootstrappers loader: parse operator-supplied JSON list of
//! `(Ed25519 peer pubkey, host:port)` pairs for cold-start peer discovery.
//!
//! No in-tree per-chain default lists — operator MUST provide a JSON file
//! via `--dpos.bootstrappers` (genesis bootstrap event = empty `[]`
//! JSON file). This avoids:
//! - Silent prod deployment with empty defaults (no defense before).
//! - Chain-ID duplication between `bootstrappers.rs` and `chainspec.rs`.
//! - In-tree placeholder lists that drift from foundation's deployed bootnodes.
//!
//! Format normalization contract: see [`load_from_json_path`].

use std::time::Duration;

use commonware_codec::DecodeExt as _;
use commonware_p2p::authenticated::discovery::Bootstrapper;
use fluentbase_bls::PeerPubkey;
use hickory_resolver::TokioAsyncResolver;
use tokio::time::{sleep, Instant};

use crate::ingress::parse_ingress;

/// `--dpos.bootstrappers` value dispatch: a `dns:<domain>` prefix selects DNS
/// TXT seed discovery; anything else is a JSON file path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootstrapperSpec<'a> {
    /// Filesystem path to the bootstrappers JSON list (existing behavior).
    JsonPath(&'a str),
    /// DNS domain whose TXT records each carry one `pubkey@host:port` seed.
    Dns(&'a str),
}

/// Classify a `--dpos.bootstrappers` value. `dns:<domain>` → [`BootstrapperSpec::Dns`];
/// any other string → [`BootstrapperSpec::JsonPath`] (a filesystem path).
pub fn classify_spec(spec: &str) -> BootstrapperSpec<'_> {
    match spec.strip_prefix("dns:") {
        Some(domain) => BootstrapperSpec::Dns(domain),
        None => BootstrapperSpec::JsonPath(spec),
    }
}

/// Backoff schedule for the DNS TXT retry loop.
#[derive(Copy, Clone, Debug)]
struct RetrySchedule {
    /// First inter-attempt delay.
    initial: Duration,
    /// Cap the (doubling) delay never exceeds.
    max: Duration,
    /// Total window after which the loop STOPS retrying and proceeds with
    /// whatever it has (possibly zero).
    window: Duration,
}

/// Production DNS TXT retry: 2 s initial, doubling to a 30 s cap, giving up after
/// ~2 min (covers a seed DNS service starting a little after the validator).
const DNS_RETRY: RetrySchedule = RetrySchedule {
    initial: Duration::from_secs(2),
    max: Duration::from_secs(30),
    window: Duration::from_secs(120),
};

/// Parse one TXT record `"pubkey_hex@host:port"` into a bootstrapper.
///
/// - `pubkey_hex`: `0x`-optional lowercase hex of a 32-byte Ed25519 peer pubkey,
///   subgroup-checked at decode (same as the JSON-path `peer_pubkey`).
/// - `host:port`: parsed by [`parse_ingress`] — an IP literal or (under
///   `ALLOW_DNS`) a DNS hostname.
///
/// Errors name the offending record. Split on the FIRST `@` (a pubkey hex never
/// contains `@`, so this is unambiguous).
pub fn parse_txt_record(record: &str) -> eyre::Result<Bootstrapper<PeerPubkey>> {
    let record = record.trim();
    let (pubkey_hex, addr) = record.split_once('@').ok_or_else(|| {
        eyre::eyre!("bootstrapper TXT record {record:?} is missing '@' (expected pubkey@host:port)")
    })?;
    let bytes = commonware_utils::from_hex_formatted(pubkey_hex.trim())
        .ok_or_else(|| eyre::eyre!("bootstrapper TXT record {record:?} has a non-hex pubkey"))?;
    let pk = PeerPubkey::decode(bytes.as_slice())
        .map_err(|e| eyre::eyre!("bootstrapper TXT record {record:?} pubkey decode failed: {e:?}"))?;
    let ingress = parse_ingress(addr.trim())
        .map_err(|e| eyre::eyre!("bootstrapper TXT record {record:?} address parse failed: {e}"))?;
    Ok((pk, ingress))
}

/// Load bootstrappers from the TXT records of a DNS `domain`.
///
/// Each TXT record is one `pubkey@host:port` seed ([`parse_txt_record`]). A
/// malformed record is warned-and-skipped. A lookup error OR zero valid records
/// is retried with exponential backoff ([`DNS_RETRY`]); each attempt logs at warn.
///
/// **Non-fatal on failure.** If the window elapses with no valid records, this
/// logs one prominent warn and returns whatever it obtained — possibly an EMPTY
/// list. A seed node itself has no one to dial and must still come up when its
/// DNS zone is down: inbound connections + discovery gossip work with an empty
/// bootstrapper list. Only a failure to READ the system resolver config
/// (`/etc/resolv.conf`) is a hard error (genuine environment misconfig).
///
/// Uses the system resolver config; must be called on a tokio runtime. Note: a
/// listed seed must be a committee-tracking validator (commonware only serves
/// peer Info for tracked peers), not an arbitrary host.
pub async fn load_from_dns(domain: &str) -> eyre::Result<Vec<Bootstrapper<PeerPubkey>>> {
    let (config, mut opts) = hickory_resolver::system_conf::read_system_conf().map_err(|e| {
        eyre::eyre!("failed reading system DNS config for bootstrappers domain {domain:?}: {e}")
    })?;
    // Disable the resolver's positive+negative cache for this one-shot retry loop.
    // With caching on, an authoritative NXDOMAIN (a wrong-record-name misconfig)
    // is cached for the zone's negative TTL, so every subsequent retry inside the
    // ~2 min window is a cache hit — degrading the retry to a single real query.
    // cache_size = 0 forces each attempt to re-query, so a late-starting seed
    // (refused/SERVFAIL/timeout — never cached) is genuinely re-tried.
    opts.cache_size = 0;
    let resolver = TokioAsyncResolver::tokio(config, opts);
    Ok(drive_txt_retry(domain, DNS_RETRY, || resolve_txt(&resolver, domain)).await)
}

/// Drive the retry loop over an injectable `lookup` (real resolver in prod, a
/// stub in tests). Returns the first non-empty result, or — once `schedule.window`
/// elapses — whatever the last attempt yielded (possibly empty). Never errors:
/// an empty bootstrapper list is a valid inbound-only startup state.
async fn drive_txt_retry<F, Fut>(
    domain: &str,
    schedule: RetrySchedule,
    mut lookup: F,
) -> Vec<Bootstrapper<PeerPubkey>>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = eyre::Result<Vec<Bootstrapper<PeerPubkey>>>>,
{
    let deadline = Instant::now() + schedule.window;
    let mut backoff = schedule.initial;
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        match lookup().await {
            Ok(bootstrappers) if !bootstrappers.is_empty() => {
                tracing::info!(
                    domain,
                    attempt,
                    count = bootstrappers.len(),
                    "loaded bootstrappers from DNS TXT"
                );
                return bootstrappers;
            }
            Ok(_) => tracing::warn!(
                domain,
                attempt,
                "bootstrappers DNS TXT lookup returned zero valid records; retrying"
            ),
            Err(e) => tracing::warn!(
                domain,
                attempt,
                error = %e,
                "bootstrappers DNS TXT lookup failed; retrying"
            ),
        }
        if Instant::now() >= deadline {
            tracing::warn!(
                "starting with 0 bootstrappers; DNS seed {domain:?} unavailable/empty \
                 — node is inbound-only until peers are discovered"
            );
            return Vec::new();
        }
        sleep(backoff).await;
        backoff = (backoff * 2).min(schedule.max);
    }
}

/// One TXT lookup + parse pass. Malformed records are warned-and-skipped; the
/// returned vec holds only the well-formed seeds.
async fn resolve_txt(
    resolver: &TokioAsyncResolver,
    domain: &str,
) -> eyre::Result<Vec<Bootstrapper<PeerPubkey>>> {
    let lookup = resolver
        .txt_lookup(domain)
        .await
        .map_err(|e| eyre::eyre!("TXT lookup failed: {e}"))?;
    let mut out = Vec::new();
    for txt in lookup.iter() {
        // A TXT RR is a sequence of character-strings; concatenate them.
        let joined: Vec<u8> = txt.txt_data().iter().flat_map(|s| s.iter().copied()).collect();
        let record = String::from_utf8_lossy(&joined);
        match parse_txt_record(&record) {
            Ok(bootstrapper) => out.push(bootstrapper),
            Err(e) => {
                tracing::warn!(record = %record, "skipping malformed bootstrapper TXT record: {e}")
            }
        }
    }
    Ok(out)
}

/// JSON schema for [`load_from_json_path`] (de-by-serde).
#[derive(serde::Deserialize)]
struct BootstrapperJson {
    /// Hex-encoded Ed25519 peer pubkey (32 bytes); accepts `0x`-prefixed or bare.
    peer_pubkey: String,
    /// Peer ingress address as `"host:port"` — an IP literal (e.g.
    /// `"10.0.0.1:9000"`, `"[::1]:9000"`) or, when the network's `ALLOW_DNS`
    /// policy permits, a DNS hostname (e.g. `"validator-3:9000"`). Parsed by
    /// [`parse_ingress`].
    socket: String,
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
/// - `socket`: a `"host:port"` string. An IP literal parses to
///   `Ingress::Socket`; IPv6 addresses **require brackets** (e.g.
///   `"[::1]:9000"`; bare `"::1:9000"` rejects). A non-IP host is treated as a
///   DNS hostname (RFC-1035/1123) yielding `Ingress::Dns` — only *dialed* when
///   the network-wide `ALLOW_DNS` policy is `true` (see `constants::ALLOW_DNS`).
///   Existing IP-form files parse identically to before this change.
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
            let ingress = parse_ingress(&e.socket)
                .map_err(|err| eyre::eyre!("bootstrapper socket parse failed: {err}"))?;
            Ok((pk, ingress))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use commonware_p2p::Ingress;

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
    fn load_from_json_path_accepts_dns_hostname_socket() {
        use commonware_codec::Encode as _;
        use commonware_cryptography::{ed25519::PrivateKey, Signer};
        use commonware_math::algebra::Random as _;
        use rand_core::SeedableRng as _;
        let mut rng = rand_08::rngs::StdRng::seed_from_u64(7);
        let sk = PrivateKey::random(&mut rng);
        let hex = hex::encode(sk.public_key().encode().as_ref());
        let json = format!(r#"[{{"peer_pubkey":"0x{hex}","socket":"validator-3:9000"}}]"#);
        let dir = std::env::temp_dir();
        let path = dir.join(format!("bootstrappers_test_dns_{}.json", std::process::id()));
        std::fs::write(&path, json).unwrap();

        let loaded = load_from_json_path(&path).expect("dns socket");
        assert_eq!(loaded.len(), 1);
        match &loaded[0].1 {
            Ingress::Dns { host, port } => {
                assert_eq!(host.as_str(), "validator-3");
                assert_eq!(*port, 9000);
            }
            other => panic!("expected Dns ingress, got {other:?}"),
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

    /// Derive a deterministic Ed25519 peer pubkey hex for TXT-record tests.
    fn test_pubkey_hex(seed: u64) -> String {
        use commonware_codec::Encode as _;
        use commonware_cryptography::{ed25519::PrivateKey, Signer};
        use commonware_math::algebra::Random as _;
        use rand_core::SeedableRng as _;
        let mut rng = rand_08::rngs::StdRng::seed_from_u64(seed);
        let sk = PrivateKey::random(&mut rng);
        hex::encode(sk.public_key().encode().as_ref())
    }

    #[test]
    fn parse_txt_record_ip_form() {
        let hex = test_pubkey_hex(1);
        let (_, ingress) = parse_txt_record(&format!("0x{hex}@10.0.0.1:9000")).unwrap();
        assert_eq!(ingress, Ingress::Socket("10.0.0.1:9000".parse().unwrap()));
    }

    #[test]
    fn parse_txt_record_hostname_form() {
        let hex = test_pubkey_hex(2);
        let (_, ingress) = parse_txt_record(&format!("{hex}@validator-2:9000")).unwrap();
        match ingress {
            Ingress::Dns { host, port } => {
                assert_eq!(host.as_str(), "validator-2");
                assert_eq!(port, 9000);
            }
            other => panic!("expected Dns, got {other:?}"),
        }
    }

    #[test]
    fn parse_txt_record_accepts_bare_and_prefixed_hex() {
        let hex = test_pubkey_hex(3);
        let bare = parse_txt_record(&format!("{hex}@10.0.0.1:9000")).unwrap();
        let prefixed = parse_txt_record(&format!("0x{hex}@10.0.0.1:9000")).unwrap();
        use commonware_codec::Encode as _;
        assert_eq!(bare.0.encode().as_ref(), prefixed.0.encode().as_ref());
    }

    #[test]
    fn parse_txt_record_rejects_bad_hex() {
        assert!(parse_txt_record("zzzz@10.0.0.1:9000").is_err());
    }

    #[test]
    fn parse_txt_record_rejects_short_pubkey() {
        assert!(parse_txt_record("0xdeadbeef@10.0.0.1:9000").is_err());
    }

    #[test]
    fn parse_txt_record_rejects_missing_at() {
        let hex = test_pubkey_hex(4);
        assert!(parse_txt_record(&format!("{hex}_10.0.0.1:9000")).is_err());
    }

    #[test]
    fn parse_txt_record_rejects_bad_port() {
        let hex = test_pubkey_hex(5);
        assert!(parse_txt_record(&format!("0x{hex}@validator-2:notaport")).is_err());
    }

    #[test]
    fn parse_txt_record_rejects_empty() {
        assert!(parse_txt_record("").is_err());
        assert!(parse_txt_record("   ").is_err());
    }

    #[tokio::test]
    async fn drive_txt_retry_empty_after_window_returns_empty_not_err() {
        // Window ZERO => exactly one attempt, then proceed with what we have.
        let schedule = RetrySchedule {
            initial: Duration::from_millis(0),
            max: Duration::from_millis(0),
            window: Duration::from_millis(0),
        };
        let out = drive_txt_retry("seed.soak.local", schedule, || async {
            Err::<Vec<Bootstrapper<PeerPubkey>>, _>(eyre::eyre!("simulated SERVFAIL"))
        })
        .await;
        assert!(out.is_empty(), "lookup failure must yield an empty list, not an error");
    }

    #[tokio::test]
    async fn drive_txt_retry_zero_records_after_window_returns_empty() {
        let schedule = RetrySchedule {
            initial: Duration::from_millis(0),
            max: Duration::from_millis(0),
            window: Duration::from_millis(0),
        };
        let out =
            drive_txt_retry("seed.soak.local", schedule, || async { Ok(Vec::new()) }).await;
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn drive_txt_retry_returns_first_nonempty() {
        let hex = test_pubkey_hex(9);
        let seed = parse_txt_record(&format!("0x{hex}@10.0.0.1:9000")).unwrap();
        let schedule = RetrySchedule {
            initial: Duration::from_millis(0),
            max: Duration::from_millis(0),
            window: Duration::from_secs(60),
        };
        let seed = std::sync::Mutex::new(Some(seed));
        let out = drive_txt_retry("seed.soak.local", schedule, || {
            let taken = seed.lock().unwrap().take();
            async move { Ok(taken.into_iter().collect::<Vec<_>>()) }
        })
        .await;
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn classify_spec_dispatches_on_dns_prefix() {
        assert_eq!(
            classify_spec("dns:seed.soak.local"),
            BootstrapperSpec::Dns("seed.soak.local")
        );
        assert_eq!(classify_spec("dns:"), BootstrapperSpec::Dns(""));
        assert_eq!(
            classify_spec("/runtime/peers.json"),
            BootstrapperSpec::JsonPath("/runtime/peers.json")
        );
        // A path that merely contains "dns" but lacks the prefix stays a path.
        assert_eq!(
            classify_spec("/etc/dns/peers.json"),
            BootstrapperSpec::JsonPath("/etc/dns/peers.json")
        );
    }
}
