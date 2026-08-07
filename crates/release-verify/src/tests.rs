use crate::{
    asset::ReleaseAsset,
    error::VerifyError,
    key::{validate_certificate_policy_at, ReleaseKey, FLUENT_RELEASE_KEY_FINGERPRINT},
    load::{authenticate, load_verified, parse_genesis_gz, read_capped, VerifiedArtifact},
    manifest::ReleaseManifest,
    signature::{signing_candidates, verify_detached_signature, ACCEPTED_HASH_ALGORITHMS},
    test_support::{gzip, offline, plant_cache, FakeRelease, TestKey},
    MAX_SIGNATURE_BYTES,
};
use pgp::{
    composed::{Deserializable as _, DetachedSignature, SignedPublicKey},
    crypto::hash::HashAlgorithm,
    packet::{Signature, SignatureType},
};
use sha2::Digest as _;
use std::{
    fs,
    io::Cursor,
    sync::{Arc, Barrier},
};

/// Minimal but valid genesis JSON, gzipped, used as a stand-in for a release artifact.
fn sample_genesis_gz() -> Vec<u8> {
    gzip(br#"{"config":{"chainId":1337},"alloc":{},"gasLimit":"0x1c9c380","difficulty":"0x0"}"#)
}

/// A genesis an attacker would like the caller to use instead.
fn substituted_genesis_gz() -> Vec<u8> {
    gzip(br#"{"config":{"chainId":31337},"alloc":{},"gasLimit":"0x1","difficulty":"0x0"}"#)
}

fn armor(sig: DetachedSignature) -> Vec<u8> {
    sig.to_armored_bytes(Default::default())
        .expect("armor signature")
}

fn parse_sig(asc: &[u8]) -> DetachedSignature {
    DetachedSignature::from_armor_single(Cursor::new(asc))
        .expect("parse signature")
        .0
}

fn asset() -> ReleaseAsset {
    ReleaseAsset::genesis("v9.9.9", None).expect("valid test asset")
}

#[test]
fn release_asset_rejects_path_and_url_metacharacters() {
    for tag in ["", "../v1.0.0", "v1.0.0?download=1", "release/name"] {
        let err = ReleaseAsset::genesis(tag.to_owned(), None)
            .expect_err("an invalid release tag must be rejected before URL construction");
        assert!(matches!(err, VerifyError::Asset(_)), "{tag:?}: {err}");
    }

    for channel in ["../mainnet", "mainnet#fragment", "main/net"] {
        let err = ReleaseAsset::genesis("v1.0.0", Some(channel))
            .expect_err("an invalid channel must be rejected before filename construction");
        assert!(matches!(err, VerifyError::Asset(_)), "{channel:?}: {err}");
    }

    for name in ["../artifact", "/tmp/artifact", "artifact?raw=1"] {
        let err = ReleaseAsset::new("v1.0.0", name.to_owned())
            .expect_err("an invalid asset name must be rejected before cache construction");
        assert!(matches!(err, VerifyError::Asset(_)), "{name:?}: {err}");
    }
}

#[test]
fn concurrent_atomic_writers_do_not_share_a_temporary_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("artifact");
    let barrier = Arc::new(Barrier::new(3));
    let payloads = [vec![0x11; 1024 * 1024], vec![0x22; 1024 * 1024]];
    let expected_payloads = payloads.clone();
    let writers: Vec<_> = payloads
        .into_iter()
        .map(|payload| {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                crate::write_atomic(&path, &payload)
            })
        })
        .collect();

    barrier.wait();
    for writer in writers {
        writer
            .join()
            .expect("writer thread must not panic")
            .expect("concurrent atomic write must succeed");
    }

    let stored = fs::read(path).unwrap();
    assert!(expected_payloads
        .iter()
        .any(|payload| payload.as_slice() == stored.as_slice()));
}

fn verified(bytes: Vec<u8>) -> VerifiedArtifact {
    VerifiedArtifact {
        name: "genesis-v9.9.9.json.gz".to_owned(),
        sha256: <[u8; 32]>::from(sha2::Sha256::digest(&bytes)),
        bytes,
    }
}

// -------------------------------------------------------------------------
// The pinned release key
// -------------------------------------------------------------------------

#[test]
fn embedded_release_key_parses_and_matches_its_pin() {
    let key = ReleaseKey::fluent().expect("embedded key must load");
    assert_eq!(key.fingerprint(), FLUENT_RELEASE_KEY_FINGERPRINT);
    assert!(
        !signing_candidates(key.cert()).is_empty(),
        "embedded key must expose at least one signing component"
    );
}

#[test]
fn release_key_that_misses_its_fingerprint_pin_is_rejected() {
    let impostor = TestKey::new("Impostor <impostor@example.com>");
    let err = ReleaseKey::new(&impostor.armored, FLUENT_RELEASE_KEY_FINGERPRINT)
        .expect_err("must reject a key that does not match the pin");
    assert!(matches!(err, VerifyError::KeyFingerprint { .. }), "{err}");
}

#[test]
fn malformed_release_key_is_rejected() {
    let err = ReleaseKey::new("not a pgp key at all", FLUENT_RELEASE_KEY_FINGERPRINT)
        .expect_err("must reject garbage key");
    assert!(matches!(err, VerifyError::KeyParse(_)), "{err}");
}

#[test]
fn release_key_policy_rejects_a_revoked_primary_key() {
    let release = TestKey::release();
    let (mut cert, _) = SignedPublicKey::from_string(&release.armored).unwrap();
    let simulated_revocation = cert.details.users[0].signatures[0].clone();
    cert.details
        .revocation_signatures
        .push(simulated_revocation);

    let err = validate_certificate_policy_at(&cert, u64::MAX)
        .expect_err("a certificate snapshot containing a primary revocation must be rejected");
    assert!(matches!(err, VerifyError::KeyPolicy(_)), "{err}");
}

// -------------------------------------------------------------------------
// Signature verification
// -------------------------------------------------------------------------

#[test]
fn valid_signature_is_accepted() {
    let release = TestKey::release();
    let data = sample_genesis_gz();
    let sig = release.sign(&data);

    verify_detached_signature(&data, &sig, &release.key()).expect("valid signature must verify");
}

#[test]
fn signature_over_different_content_is_rejected() {
    let release = TestKey::release();
    let data = sample_genesis_gz();
    let sig = release.sign(&data);

    let mut tampered = data.clone();
    *tampered.last_mut().unwrap() ^= 0x01;

    let err = verify_detached_signature(&tampered, &sig, &release.key())
        .expect_err("content mismatch must be rejected");
    assert!(matches!(err, VerifyError::BadSignature(_)), "{err}");
}

#[test]
fn signature_from_a_different_key_is_rejected() {
    let release = TestKey::release();
    let attacker = TestKey::attacker();
    let data = sample_genesis_gz();

    let err = verify_detached_signature(&data, &attacker.sign(&data), &release.key())
        .expect_err("wrong-key signature must be rejected");
    assert!(matches!(err, VerifyError::UntrustedIssuer), "{err}");
}

#[test]
fn signature_with_a_spoofed_issuer_is_rejected() {
    // Issuer subpackets are attacker-controlled hints; only the cryptographic check counts.
    let release = TestKey::release();
    let attacker = TestKey::attacker();
    let data = sample_genesis_gz();

    let mut forged = parse_sig(&attacker.sign(&data));
    let genuine = parse_sig(&release.sign(&data));
    let genuine_issuers = genuine
        .signature
        .config()
        .expect("config")
        .unhashed_subpackets
        .clone();
    for (idx, subpacket) in genuine_issuers.into_iter().enumerate() {
        forged
            .signature
            .unhashed_subpacket_insert(idx, subpacket)
            .expect("insert issuer subpacket");
    }

    let err = verify_detached_signature(&data, &armor(forged), &release.key())
        .expect_err("a forged issuer must not make a bad signature verify");
    assert!(
        matches!(
            err,
            VerifyError::BadSignature(_) | VerifyError::UntrustedIssuer
        ),
        "{err}"
    );
}

#[test]
fn empty_and_malformed_signatures_are_rejected() {
    let release = TestKey::release();
    let key = release.key();
    let data = sample_genesis_gz();

    for sig in [
        b"".to_vec(),
        b"not a signature".to_vec(),
        b"-----BEGIN PGP SIGNATURE-----\n\nzzzz\n-----END PGP SIGNATURE-----\n".to_vec(),
    ] {
        let err = verify_detached_signature(&data, &sig, &key)
            .expect_err("malformed signature must be rejected");
        assert!(matches!(err, VerifyError::SignatureParse(_)), "{err}");
    }
}

#[test]
fn truncated_signature_is_rejected() {
    let release = TestKey::release();
    let data = sample_genesis_gz();
    let mut sig = release.sign(&data);
    sig.truncate(sig.len() / 2);

    verify_detached_signature(&data, &sig, &release.key())
        .expect_err("truncated signature must be rejected");
}

#[test]
fn public_key_block_in_place_of_a_signature_is_rejected() {
    let release = TestKey::release();
    let data = sample_genesis_gz();

    verify_detached_signature(&data, release.armored.as_bytes(), &release.key())
        .expect_err("a certificate is not a detached signature");
}

#[test]
fn weak_digest_signature_is_rejected() {
    let release = TestKey::release();
    let data = sample_genesis_gz();

    // rPGP refuses to *produce* a SHA-1 EdDSA signature, so downgrade a genuine one the way an
    // attacker betting on a weak digest would.
    let genuine = parse_sig(&release.sign(&data));
    let mut config = genuine.signature.config().expect("config").clone();
    config.hash_alg = HashAlgorithm::Sha1;
    let weak = Signature::from_config(
        config,
        genuine.signature.signed_hash_value().expect("hash value"),
        genuine.signature.signature().expect("sig bytes").clone(),
    )
    .expect("build downgraded signature");

    let err =
        verify_detached_signature(&data, &armor(DetachedSignature::new(weak)), &release.key())
            .expect_err("SHA-1 signature must be rejected");
    assert!(matches!(err, VerifyError::WeakDigest(_)), "{err}");
}

#[test]
fn text_mode_signature_is_rejected() {
    let release = TestKey::release();
    let data = sample_genesis_gz();

    let err = verify_detached_signature(&data, &release.sign_text(&data), &release.key())
        .expect_err("text mode signature must be rejected");
    assert!(matches!(err, VerifyError::SignatureType(_)), "{err}");
}

// -------------------------------------------------------------------------
// Digest pinning
// -------------------------------------------------------------------------

#[test]
fn digest_pin_mismatch_is_rejected_even_with_a_valid_signature() {
    let release = TestKey::release();
    let data = sample_genesis_gz();
    let sig = release.sign(&data);

    let err = authenticate(
        &data,
        &sig,
        &asset().with_sha256([0x11; 32]),
        &release.key(),
    )
    .expect_err("digest pin must apply");
    assert!(matches!(err, VerifyError::DigestMismatch { .. }), "{err}");
}

#[test]
fn matching_digest_pin_is_accepted() {
    let release = TestKey::release();
    let data = sample_genesis_gz();
    let sig = release.sign(&data);
    let digest: [u8; 32] = sha2::Sha256::digest(&data).into();

    let artifact = authenticate(&data, &sig, &asset().with_sha256(digest), &release.key())
        .expect("pinned digest must verify");
    assert_eq!(artifact.sha256, digest);
    assert_eq!(artifact.bytes, data);
}

// -------------------------------------------------------------------------
// Cache handling
// -------------------------------------------------------------------------

#[test]
fn download_verifies_then_caches_and_reuses_the_cache() {
    let release = TestKey::release();
    let dir = tempfile::tempdir().unwrap();
    let bytes = sample_genesis_gz();
    let asc = release.sign(&bytes);
    let digest: [u8; 32] = sha2::Sha256::digest(&bytes).into();
    let asset = asset().with_sha256(digest);
    let feed = FakeRelease::new().publish(&asset, bytes.clone(), asc.clone());

    let artifact = load_verified(Some(dir.path()), &asset, &release.key(), &|url, max| {
        feed.fetch(url, max)
    })
    .expect("first load must succeed");
    assert_eq!(artifact.bytes, bytes);
    assert_eq!(feed.calls(), 2);
    assert_eq!(fs::read(dir.path().join(asset.name())).unwrap(), bytes);
    assert_eq!(
        fs::read(dir.path().join(asset.signature_name())).unwrap(),
        asc
    );

    // Second load is served from cache: the fetcher is not touched again.
    load_verified(Some(dir.path()), &asset, &release.key(), &|url, max| {
        feed.fetch(url, max)
    })
    .expect("cached load must succeed");
    assert_eq!(feed.calls(), 2);
}

#[test]
fn replaced_cache_is_discarded_and_refetched() {
    let release = TestKey::release();
    let attacker = TestKey::attacker();
    let dir = tempfile::tempdir().unwrap();
    let bytes = sample_genesis_gz();
    let asset = asset();
    let feed = FakeRelease::new().publish(&asset, bytes.clone(), release.sign(&bytes));

    let evil = substituted_genesis_gz();
    plant_cache(dir.path(), &asset, &evil, &attacker.sign(&evil));

    let artifact = load_verified(Some(dir.path()), &asset, &release.key(), &|url, max| {
        feed.fetch(url, max)
    })
    .expect("must fall back to the release");

    // The substituted bytes never reach the caller, and the cache is repaired.
    assert_eq!(artifact.bytes, bytes);
    assert_eq!(parse_genesis_gz(&artifact).unwrap().config.chain_id, 1337);
    assert_eq!(feed.calls(), 2);
    assert_eq!(fs::read(dir.path().join(asset.name())).unwrap(), bytes);
}

#[test]
fn replaced_cache_is_not_accepted_when_the_release_is_unreachable() {
    let release = TestKey::release();
    let attacker = TestKey::attacker();
    let dir = tempfile::tempdir().unwrap();
    let asset = asset();

    let evil = substituted_genesis_gz();
    plant_cache(dir.path(), &asset, &evil, &attacker.sign(&evil));

    load_verified(Some(dir.path()), &asset, &release.key(), &offline)
        .expect_err("an unauthenticated cache must never be used");
}

#[test]
fn same_name_cache_substitution_is_rejected() {
    // The classic attack this guards: drop a file with the expected name into the cache directory
    // and let the next run pick it up.
    let release = TestKey::release();
    let dir = tempfile::tempdir().unwrap();
    let asset = asset();
    let genuine = sample_genesis_gz();

    // A genuine signature, but next to bytes it does not cover.
    plant_cache(
        dir.path(),
        &asset,
        &substituted_genesis_gz(),
        &release.sign(&genuine),
    );

    let err = load_verified(Some(dir.path()), &asset, &release.key(), &offline)
        .expect_err("a signature from another artifact must not authenticate these bytes");
    assert!(matches!(err, VerifyError::Fetch { .. }), "{err}");
    assert!(
        !dir.path().join(asset.name()).exists(),
        "the rejected cache entry must be gone"
    );
}

#[test]
fn cache_with_a_missing_signature_is_not_accepted() {
    let release = TestKey::release();
    let dir = tempfile::tempdir().unwrap();
    let asset = asset();
    let bytes = sample_genesis_gz();
    fs::write(dir.path().join(asset.name()), &bytes).unwrap();

    // Cached data with no signature next to it is not trusted, even though the bytes are genuine.
    load_verified(Some(dir.path()), &asset, &release.key(), &offline)
        .expect_err("missing signature must fail closed");

    // With the release reachable the same bytes are accepted only after re-fetching.
    let feed = FakeRelease::new().publish(&asset, bytes.clone(), release.sign(&bytes));
    load_verified(Some(dir.path()), &asset, &release.key(), &|url, max| {
        feed.fetch(url, max)
    })
    .expect("re-fetch must succeed");
    assert_eq!(feed.calls(), 2);
}

#[test]
fn downloaded_artifact_that_fails_verification_is_not_cached() {
    let release = TestKey::release();
    let attacker = TestKey::attacker();
    let dir = tempfile::tempdir().unwrap();
    let asset = asset();
    let bytes = sample_genesis_gz();
    let feed = FakeRelease::new().publish(&asset, bytes.clone(), attacker.sign(&bytes));

    load_verified(Some(dir.path()), &asset, &release.key(), &|url, max| {
        feed.fetch(url, max)
    })
    .expect_err("a badly signed download must fail closed");

    assert!(!dir.path().join(asset.name()).exists());
    assert!(!dir.path().join(asset.signature_name()).exists());
}

#[test]
fn loading_without_a_cache_still_verifies() {
    let release = TestKey::release();
    let attacker = TestKey::attacker();
    let asset = asset();
    let bytes = sample_genesis_gz();

    let good = FakeRelease::new().publish(&asset, bytes.clone(), release.sign(&bytes));
    load_verified(None, &asset, &release.key(), &|url, max| {
        good.fetch(url, max)
    })
    .expect("uncached load must succeed");

    let bad = FakeRelease::new().publish(&asset, bytes.clone(), attacker.sign(&bytes));
    load_verified(None, &asset, &release.key(), &|url, max| {
        bad.fetch(url, max)
    })
    .expect_err("uncached load must still fail closed");
}

#[test]
fn oversized_cached_artifact_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let asset = asset();
    plant_cache(
        dir.path(),
        &asset,
        &sample_genesis_gz(),
        &vec![0u8; MAX_SIGNATURE_BYTES + 1],
    );

    let err = read_capped(
        &dir.path().join(asset.signature_name()),
        MAX_SIGNATURE_BYTES,
    )
    .expect_err("oversized signature must be rejected");
    assert!(matches!(err, VerifyError::TooLarge { .. }), "{err}");
}

// -------------------------------------------------------------------------
// Release manifest
// -------------------------------------------------------------------------

const SAMPLE_MANIFEST: &str = "version=v1.3.2
commit=d6d8d2e739f50daa8174b299cb5170a9ce7e7974

[raw]
4fcdd3610da44467e709972b638097cd2bdb5377dd8adb792e8c2328c1e41dc0  ./crates/genesis/genesis-devnet.json

[compressed]
92704da98369998447aae3a4bca614b1e1b5f989e9c05f12d300dc933ffade73  ./artifacts/genesis-v1.3.2.json.gz
3086b3bf0ceeb1ea7ebb5ea5a523a12df72c100f2d723fcf75486a7e1a7382d1  ./artifacts/genesis-mainnet-v1.3.2.json.gz
";

fn sample_manifest() -> ReleaseManifest {
    ReleaseManifest::parse(SAMPLE_MANIFEST.as_bytes()).expect("manifest must parse")
}

fn digest(hex_digest: &str) -> [u8; 32] {
    hex::decode(hex_digest).unwrap().try_into().unwrap()
}

#[test]
fn manifest_parses_release_metadata_and_digests() {
    let manifest = sample_manifest();
    assert_eq!(manifest.version(), "v1.3.2");
    assert_eq!(
        manifest.commit(),
        "d6d8d2e739f50daa8174b299cb5170a9ce7e7974"
    );
    assert_eq!(
        manifest.digest_for("genesis-v1.3.2.json.gz"),
        Some(digest(
            "92704da98369998447aae3a4bca614b1e1b5f989e9c05f12d300dc933ffade73"
        ))
    );
    assert_eq!(manifest.digest_for("genesis-v9.9.9.json.gz"), None);
}

#[test]
fn manifest_accepts_the_artifact_it_vouches_for() {
    sample_manifest()
        .check(
            "v1.3.2",
            "genesis-v1.3.2.json.gz",
            &digest("92704da98369998447aae3a4bca614b1e1b5f989e9c05f12d300dc933ffade73"),
        )
        .expect("listed digest must be accepted");
}

#[test]
fn manifest_rejects_a_modified_artifact() {
    let err = sample_manifest()
        .check("v1.3.2", "genesis-v1.3.2.json.gz", &[0x11; 32])
        .expect_err("modified artifact must be rejected");
    assert!(matches!(err, VerifyError::DigestMismatch { .. }), "{err}");
}

#[test]
fn manifest_from_another_release_is_rejected() {
    // Replaying a validly signed manifest from a different release must not bind this one.
    let err = sample_manifest()
        .check(
            "v1.3.3",
            "genesis-v1.3.2.json.gz",
            &digest("92704da98369998447aae3a4bca614b1e1b5f989e9c05f12d300dc933ffade73"),
        )
        .expect_err("manifest for another release must be rejected");
    assert!(matches!(err, VerifyError::Manifest(_)), "{err}");
}

#[test]
fn manifest_that_does_not_list_the_asset_is_rejected() {
    // The mainnet/devnet split lives in the file name, so asking for the wrong network's asset
    // against a manifest that omits it must fail rather than fall through.
    let err = sample_manifest()
        .check("v1.3.2", "genesis-testnet-v1.3.2.json.gz", &[0x11; 32])
        .expect_err("unlisted asset must be rejected");
    assert!(matches!(err, VerifyError::Manifest(_)), "{err}");
}

#[test]
fn malformed_manifests_are_rejected() {
    for bad in [
        "",
        "commit=abc\n",
        "version=v1\n",
        "version=v1\ncommit=abc\nnot-a-digest  ./artifacts/x.gz\n",
        "version=v1\ncommit=abc\nzzzz  ./artifacts/x.gz\n",
    ] {
        ReleaseManifest::parse(bad.as_bytes()).expect_err(&format!("must reject manifest {bad:?}"));
    }
}

#[test]
fn manifest_with_conflicting_digests_for_one_name_is_rejected() {
    let manifest = format!(
        "version=v1\ncommit=abc\n{}  ./a/x.gz\n{}  ./b/x.gz\n",
        hex::encode([0x11; 32]),
        hex::encode([0x22; 32])
    );
    let err = ReleaseManifest::parse(manifest.as_bytes())
        .expect_err("conflicting digests must be rejected");
    assert!(matches!(err, VerifyError::Manifest(_)), "{err}");
}

#[test]
fn manifest_with_duplicate_release_metadata_is_rejected() {
    for duplicate in [
        "version=v1.3.2\nversion=v1.3.2\ncommit=deadbeef\n",
        "version=v1.3.2\ncommit=deadbeef\ncommit=deadbeef\n",
    ] {
        let err = ReleaseManifest::parse(duplicate.as_bytes())
            .expect_err("release metadata must be unique even when repeated values agree");
        assert!(matches!(err, VerifyError::Manifest(_)), "{err}");
    }
}

// -------------------------------------------------------------------------
// Genesis parsing
// -------------------------------------------------------------------------

#[test]
fn authenticated_genesis_parses() {
    let genesis = parse_genesis_gz(&verified(sample_genesis_gz())).expect("must parse");
    assert_eq!(genesis.config.chain_id, 1337);
}

#[test]
fn non_gzip_and_non_json_payloads_are_rejected() {
    parse_genesis_gz(&verified(b"not gzip".to_vec())).expect_err("must reject non-gzip");
    parse_genesis_gz(&verified(gzip(b"not json"))).expect_err("must reject non-json");
}

// -------------------------------------------------------------------------
// The published artifacts
// -------------------------------------------------------------------------

/// Detached signatures actually published by the release workflow.
const PUBLISHED_SIGNATURES: &[(&str, &[u8])] = &[
    (
        "genesis-v0.5.7.json.gz",
        include_bytes!("../testdata/genesis-v0.5.7.json.gz.asc"),
    ),
    (
        "genesis-v0.3.4-dev.json.gz",
        include_bytes!("../testdata/genesis-v0.3.4-dev.json.gz.asc"),
    ),
    (
        "genesis-mainnet-v1.0.0.json.gz",
        include_bytes!("../testdata/genesis-mainnet-v1.0.0.json.gz.asc"),
    ),
    (
        "genesis-manifest-v1.3.2.txt",
        include_bytes!("../testdata/genesis-manifest-v1.3.2.txt.asc"),
    ),
];

/// Ties the pinned key to production: every published signature must be a binary, strong-digest
/// signature issued by the key embedded in this crate.
#[test]
fn published_signatures_are_issued_by_the_pinned_release_key() {
    let key = ReleaseKey::fluent().expect("embedded key must load");
    let candidates = signing_candidates(key.cert());

    for (asset, asc) in PUBLISHED_SIGNATURES {
        let detached = parse_sig(asc);
        let sig = &detached.signature;
        assert_eq!(
            sig.typ(),
            Some(SignatureType::Binary),
            "{asset}: not a binary signature"
        );
        assert!(
            ACCEPTED_HASH_ALGORITHMS.contains(&sig.hash_alg().expect("digest algorithm")),
            "{asset}: unaccepted digest"
        );
        assert!(
            candidates.iter().any(|candidate| {
                sig.issuer_fingerprint()
                    .iter()
                    .any(|fpr| **fpr == candidate.fingerprint())
                    || sig
                        .issuer_key_id()
                        .iter()
                        .any(|id| **id == candidate.legacy_key_id())
            }),
            "{asset}: not issued by the pinned release key"
        );
    }
}

/// The real manifest, verbatim from the v1.3.2 release, must parse and bind its assets.
#[test]
fn published_manifest_parses_and_binds_its_assets() {
    verify_detached_signature(
        include_bytes!("../testdata/genesis-manifest-v1.3.2.txt"),
        include_bytes!("../testdata/genesis-manifest-v1.3.2.txt.asc"),
        &ReleaseKey::fluent().expect("embedded key must load"),
    )
    .expect("the published manifest signature must verify against the pinned release key");

    let manifest =
        ReleaseManifest::parse(include_bytes!("../testdata/genesis-manifest-v1.3.2.txt"))
            .expect("published manifest must parse");
    assert_eq!(manifest.version(), "v1.3.2");
    for asset in [
        "genesis-v1.3.2.json.gz",
        "genesis-mainnet-v1.3.2.json.gz",
        "evm-runtime-permissive-v1.3.2.rwasm.gz",
    ] {
        let digest = manifest
            .digest_for(asset)
            .unwrap_or_else(|| panic!("{asset} is not listed"));
        manifest
            .check("v1.3.2", asset, &digest)
            .unwrap_or_else(|err| panic!("{asset}: {err}"));
    }
}
