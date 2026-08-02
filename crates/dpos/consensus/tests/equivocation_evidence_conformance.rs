//! Cross-language equivocation-evidence conformance pin.
//!
//! Builds **real** Commonware `ConflictingNotarize` / `ConflictingFinalize`
//! / `NullifyFinalize` (signed via the shipped `bls12381_multisig` scheme
//! over a deterministic committee), `.encode()`s them, and pins the wire
//! bytes, the fields the Solidity `SimplexEvidenceDecoder` must extract,
//! and a full valid slash tuple. A Simplex consensus wire-format change
//! (byte encoding is the Commonware codec) makes this
//! FAIL LOUDLY — it is the gate for equivocation slashing. Single
//! source; self-documented (no companion markdown,
//! same convention as the eip2537/ed25519/hash_to_g1 pins). `EXPECTED`
//! is mirrored by-hand into the Solidity test in the SAME PR.
//!
//! Regenerate after a deliberate Commonware/blst bump:
//! `cargo test -p fluentbase-consensus --test equivocation_evidence_conformance -- --ignored print_corpus --nocapture`

use commonware_codec::{DecodeExt, Encode};
use commonware_consensus::{
    simplex::types::{
        ConflictingFinalize, ConflictingNotarize, Finalize, Notarize, NullifyFinalize, Proposal,
    },
    types::{Epoch, Round, View},
};
use commonware_cryptography::{
    certificate::Attestation, ed25519::PrivateKey as Ed25519PrivateKey,
    sha256::Digest as Sha256Digest, Signer,
};
use commonware_math::algebra::Random;
use commonware_utils::{ordered::BiMap, TryCollect};
use fluentbase_bls::{
    encoding::{pubkey_compressed_to_eip2537, signature_compressed_to_eip2537},
    fluent_namespace,
    keys::ValidatorBlsKeypair,
    pop::sign_pop,
    scheme::build_signer,
    BlsPubkey, PeerPubkey, Scheme, VoteScheme,
};
use rand_08::rngs::StdRng;
use rand_core::SeedableRng;

const C_MAIN: u64 = 20_994; // == L2 block.chainid (fluent_namespace base)
const COMMITTEE_N: usize = 4;
const OFFENDER: usize = 0; // index into the keypair vectors (pre-BiMap-sort)
const EPOCH: u64 = 7;
const VIEW: u64 = 42;

fn committee(seed: u64) -> (Vec<ValidatorBlsKeypair>, BiMap<PeerPubkey, BlsPubkey>) {
    let mut rng = StdRng::seed_from_u64(seed);
    let peer_sks: Vec<_> = (0..COMMITTEE_N)
        .map(|_| Ed25519PrivateKey::random(&mut rng))
        .collect();
    let bls_kps: Vec<_> = (0..COMMITTEE_N)
        .map(|_| ValidatorBlsKeypair::generate(&mut rng))
        .collect();
    let bimap: BiMap<_, _> = peer_sks
        .iter()
        .zip(bls_kps.iter())
        .map(|(p, b)| {
            (
                p.public_key(),
                BlsPubkey::decode(b.public_bytes().as_slice()).unwrap(),
            )
        })
        .try_collect()
        .unwrap();
    (bls_kps, bimap)
}

/// The committee in **generation order** (peer ed25519 pubkey 32 B,
/// bls pubkey 96 B compressed). The Solidity end-to-end test registers
/// these 4 validators + their consensus keys and `commitEpochCommittee`s
/// epoch 7; the contract sorts by peerPubkey into the Simplex committee
/// order (Commonware `BiMap` codec; pinned by
/// `ed25519_ordering_conformance`), so
/// `resolveSigner(7, signerIdx)` resolves to the offender =
/// generation-index `OFFENDER`, whose `blsPubkey` is the `pkCompressedRef`.
fn committee_dump() -> Vec<(String, String)> {
    let mut rng = StdRng::seed_from_u64(1);
    let peer: Vec<_> = (0..COMMITTEE_N)
        .map(|_| Ed25519PrivateKey::random(&mut rng).public_key())
        .collect();
    let bls: Vec<_> = (0..COMMITTEE_N)
        .map(|_| ValidatorBlsKeypair::generate(&mut rng).public_bytes())
        .collect();
    peer.iter()
        .zip(bls.iter())
        .map(|(p, b)| (hex::encode(p.as_ref()), hex::encode(b)))
        .collect()
}

/// PoP corpus for the committee keys (same generation order as
/// [`committee_dump`] / [`committee`]). `setConsensusKeys` now enforces
/// on-chain Proof-of-Possession, so the Solidity end-to-end slash test must
/// register each committee validator with a valid PoP. Returns
/// `(pop48, popUnc128, pkUnc256)` per key: `ns = fluent_namespace(C_MAIN)`,
/// DST = MinSig `PROOF_OF_POSSESSION` (the same PoP path pinned by
/// `hash_to_g1_conformance`). Extends THIS corpus (same file, same pin
/// discipline) — not a new corpus.
fn committee_pop_dump() -> Vec<(String, String, String)> {
    let (kps, _bimap) = committee(1);
    let ns = fluent_namespace(C_MAIN);
    kps.iter()
        .map(|kp| {
            let pk96: [u8; 96] = kp.public_bytes();
            let pop48 = sign_pop(kp, &ns);
            (
                hex::encode(pop48),
                hex::encode(signature_compressed_to_eip2537(&pop48).unwrap()),
                hex::encode(pubkey_compressed_to_eip2537(&pk96).unwrap()),
            )
        })
        .collect()
}

// Committed mirror of `committee_dump()` (generation order). OFFENDER is the
// signer; its bls (COMMITTEE[OFFENDER].1) is the on-chain `blsPubkey`.
const COMMITTEE: &[(&str, &str)] = &[
    ("ff87a0b0a3c7c0ce827e9cada5ff79e75a44a0633bfcb5b50f99307ddb26b337", "a0b5825c3dbaf52a1b20571d5925e944677d67b173100ea228cb38293f6e6f8d30ed9d9823250f2d7991640f4982493306b5c608826c61fe46820427bf25729c9f23127e072ddfeaf810db77af12076cdafac00980e6d4f5c2e3347053a3493d"),
    ("2bd9a6a1b725644b7bfb9de3d3ba78158dfc9cd5eedbfdda5e134f311ffd50f3", "83e76182bdcacfc5e8cc2c483cf437e0ff855e866fa1f12a04d5b29db143dd08d8c22abe45a4e41b72cc40218cc3db9c0754513723b99e2ebb290ee1ad940f22da790adf98c4472e52e6fd5a177d54b916d57af9f89b196dfcbced5195c6294a"),
    ("86f351ff4be28040d935afc005e52e04dffb657a1b4d50c74380b9f0fb23a325", "a7efe3c63ab8d46fdf7171593c406e50940e0e74649f1e838f3dfbb4c1b57cd2a3531396e52b9fa7e102e0bc115de5c70dae1e0769fec9bbfddb13005c06a40f37ccef87ddc02615ccf41d141580766255ec0487754ca98e80bf81053071594a"),
    ("c4e5ac21f4ecc13c7b2af46549e1f6cd4f58d6ed5f997bf26b1d0247cf0be93f", "959ad814eb44847948c580b80ed4c93a72a83bc71a691f5ed34de3e6ec07254c04b5920ff9c92f0eb722a4ee60a46736122220d80bed7ce06a858de220927be7d89ae48e3474bcc1fd46a42f6c36478664899ac23eaa8e23868e8922223cebad"),
]; // OFFENDER = 0

// Committed mirror of `committee_pop_dump()` (generation order):
// (pop48, popUnc128, pkUnc256). Filled verbatim from `print_corpus`.
const COMMITTEE_POP: &[(&str, &str, &str)] = &[
    ("b5ed9e1bb8a7331d67438a6e1cde8012b0866ce30c70e3c028fd38bd7b5a2efa16ef920ad860a756a56e46eb3a8cbcca", "0000000000000000000000000000000015ed9e1bb8a7331d67438a6e1cde8012b0866ce30c70e3c028fd38bd7b5a2efa16ef920ad860a756a56e46eb3a8cbcca000000000000000000000000000000000f3c42a093b3d9bec7835bfde00481258c23f2356db273f37a01dd9ade643ec4c3d64556ae693bd8416e92151b237808", "0000000000000000000000000000000006b5c608826c61fe46820427bf25729c9f23127e072ddfeaf810db77af12076cdafac00980e6d4f5c2e3347053a3493d0000000000000000000000000000000000b5825c3dbaf52a1b20571d5925e944677d67b173100ea228cb38293f6e6f8d30ed9d9823250f2d7991640f49824933000000000000000000000000000000000b70c4b3fc082f10e64460dfe75ef87099cba2e9324416e7a37a679599fb54125ab10d594c18e8315d4d257ce2c8a1630000000000000000000000000000000010df56392d11ff44f778098bcb25487e92f066af30d7cdfada087e67bf0ca9a349436b8223db8ff241d364c5898319e9"),
    ("b5573c800cf5f9bdd767738c7c348c65470aae9245cb8dc0758a7a5a52909f16f24f7a2a0d58e539be605236c87fcd5e", "0000000000000000000000000000000015573c800cf5f9bdd767738c7c348c65470aae9245cb8dc0758a7a5a52909f16f24f7a2a0d58e539be605236c87fcd5e00000000000000000000000000000000134033407922417ede483b9097fbea46c8e33b7e65b2215cd6aaf530f8bae20bcd463f4bc7966a00e04842d7f09d05e5", "000000000000000000000000000000000754513723b99e2ebb290ee1ad940f22da790adf98c4472e52e6fd5a177d54b916d57af9f89b196dfcbced5195c6294a0000000000000000000000000000000003e76182bdcacfc5e8cc2c483cf437e0ff855e866fa1f12a04d5b29db143dd08d8c22abe45a4e41b72cc40218cc3db9c000000000000000000000000000000000790c173722ae98c675b64c59454de6bf8e8c09778c7e7ce20fe862d1e24d13cab70514875078a038d3cefc1fd95a7a5000000000000000000000000000000000bc092865b16d33244fd7d90d91e8dab11d80d2b25c6a6e21cf49460d40ed0860b12db5d1b7cb3e370d0c51ee9251573"),
    ("b07181278601fe711fc5b140bdb5b811076617356d59531226a7acc2049e5adf44ed42240d2a98ca15a74658ccc4f647", "00000000000000000000000000000000107181278601fe711fc5b140bdb5b811076617356d59531226a7acc2049e5adf44ed42240d2a98ca15a74658ccc4f647000000000000000000000000000000000e01bf22a8e266b300f93d149b60cf3c31bde77bb2e968d117079d4ff9a6ecdd0fef89ddb246fefc39b72da84e592e7c", "000000000000000000000000000000000dae1e0769fec9bbfddb13005c06a40f37ccef87ddc02615ccf41d141580766255ec0487754ca98e80bf81053071594a0000000000000000000000000000000007efe3c63ab8d46fdf7171593c406e50940e0e74649f1e838f3dfbb4c1b57cd2a3531396e52b9fa7e102e0bc115de5c700000000000000000000000000000000022ee1dd2d9e8f7e1323378f46bd81b0117d1b1b4cc3846f6fa5707eb82fdef856e725abe6407c8fa9fb4263a6b1c96300000000000000000000000000000000139ed39eb9cf6f25b18c0212761eec0ba9dc2d2703559d1c85ba1eabd8afa5c99d0fa89b87a5eec0c9684907a5937de7"),
    ("8cd344ce1f254ae8a5cd2e707193486acc78a3bfcecd82b59ca73646ac993390516da42a4812c5413795368d5fa6bf40", "000000000000000000000000000000000cd344ce1f254ae8a5cd2e707193486acc78a3bfcecd82b59ca73646ac993390516da42a4812c5413795368d5fa6bf400000000000000000000000000000000008cc1d38dc6da92b614b647cd6dd4f51b1f7d3fdfccdcad3f64cc45c3ca7f0ff38660eb264e807bf143ae625d4f0ea1d", "00000000000000000000000000000000122220d80bed7ce06a858de220927be7d89ae48e3474bcc1fd46a42f6c36478664899ac23eaa8e23868e8922223cebad00000000000000000000000000000000159ad814eb44847948c580b80ed4c93a72a83bc71a691f5ed34de3e6ec07254c04b5920ff9c92f0eb722a4ee60a46736000000000000000000000000000000000df0bc707524390953f6ddaf1a869fab68cdf6fb706d2b554baa4f8dacd8929bd2345715e53b2f14f7ffa24630b3450500000000000000000000000000000000050d7a0c974f61aaf8bae8f988cff852a55738674991364d48a2d137890de16fc05b111441265afb18e2568a6f9f800a"),
];

fn digest(tag: u8) -> Sha256Digest {
    let mut d = [0u8; 32];
    d[0] = tag;
    d[31] = tag;
    Sha256Digest::from(d)
}

fn round() -> Round {
    Round::new(Epoch::new(EPOCH), View::new(VIEW))
}

/// One conformance row, built from real Simplex consensus evidence
/// (Commonware codec). `kind`:
/// 0=Notarize, 1=Nullify, 2=Finalize (→ namespace suffix + the contract's
/// per-vote handling). `msg*` is the raw signed body (Proposal/Round
/// `.encode()`); `sig*` the 48 B compressed G1; `*_unc` the EIP-2537
/// uncompressed forms the slasher supplies on-chain.
struct Vector {
    label: &'static str,
    evidence: String,
    epoch: u64,
    signer_idx: u32,
    kind1: u8,
    msg1: String,
    sig1: String,
    kind2: u8,
    msg2: String,
    sig2: String,
    pk_unc: String,
    sig1_unc: String,
    sig2_unc: String,
}

/// Committed mirror of [`Vector`] (hex copied verbatim from `print_corpus`).
struct Expected {
    label: &'static str,
    evidence: &'static str,
    epoch: u64,
    signer_idx: u32,
    kind1: u8,
    msg1: &'static str,
    sig1: &'static str,
    kind2: u8,
    msg2: &'static str,
    sig2: &'static str,
    pk_unc: &'static str,
    sig1_unc: &'static str,
    sig2_unc: &'static str,
}

fn sig48(att_sig_bytes: &[u8]) -> [u8; 48] {
    <[u8; 48]>::try_from(att_sig_bytes).expect("G1 sig is 48 B")
}

/// Project a combined-scheme attestation onto its `VoteScheme` (48-B multisig)
/// half — the form that is actually submitted on-chain. This is an INDEPENDENT
/// re-implementation of `slasher::evidence::vote_attestation`: the fixture
/// builds the on-chain wire bytes this way, while the production extractor
/// reaches them by decoding the raw combined blob — both must agree.
fn vote_att(att: &Attestation<Scheme>) -> Attestation<VoteScheme> {
    Attestation {
        signer: att.signer,
        signature: (*att.signature.get().unwrap().vote()).into(),
    }
}

fn conflicting_notarize() -> (
    Vector,
    ConflictingNotarize<fluentbase_bls::Scheme, Sha256Digest>,
    BiMap<PeerPubkey, BlsPubkey>,
) {
    let (kps, bimap) = committee(1);
    let s = build_signer(
        &fluent_namespace(C_MAIN),
        bimap.clone(),
        &kps[OFFENDER],
        None,
    )
    .expect("member");
    let p1: Proposal<Sha256Digest> = Proposal::new(round(), View::new(VIEW - 1), digest(0xaa));
    let p2: Proposal<Sha256Digest> = Proposal::new(round(), View::new(VIEW - 1), digest(0xbb));
    let n1 = Notarize::sign(&s, p1.clone()).expect("sign n1");
    let n2 = Notarize::sign(&s, p2.clone()).expect("sign n2");
    let signer_idx = n1.attestation.signer.get();
    let s1 = sig48(
        n1.attestation
            .signature
            .get()
            .unwrap()
            .vote()
            .encode()
            .as_ref(),
    );
    let s2 = sig48(
        n2.attestation
            .signature
            .get()
            .unwrap()
            .vote()
            .encode()
            .as_ref(),
    );
    let on_chain = ConflictingNotarize::<VoteScheme, _>::new(
        Notarize {
            proposal: p1.clone(),
            attestation: vote_att(&n1.attestation),
        },
        Notarize {
            proposal: p2.clone(),
            attestation: vote_att(&n2.attestation),
        },
    );
    let evidence = hex::encode(on_chain.encode());
    let ev = ConflictingNotarize::new(n1, n2);
    let pk96: [u8; 96] = kps[OFFENDER].public_bytes();
    let vector = Vector {
        label: "conflicting_notarize",
        evidence,
        epoch: EPOCH,
        signer_idx,
        kind1: 0,
        msg1: hex::encode(p1.encode()),
        sig1: hex::encode(s1),
        kind2: 0,
        msg2: hex::encode(p2.encode()),
        sig2: hex::encode(s2),
        pk_unc: hex::encode(pubkey_compressed_to_eip2537(&pk96).unwrap()),
        sig1_unc: hex::encode(signature_compressed_to_eip2537(&s1).unwrap()),
        sig2_unc: hex::encode(signature_compressed_to_eip2537(&s2).unwrap()),
    };
    (vector, ev, bimap)
}

fn conflicting_finalize() -> (
    Vector,
    ConflictingFinalize<fluentbase_bls::Scheme, Sha256Digest>,
    BiMap<PeerPubkey, BlsPubkey>,
) {
    let (kps, bimap) = committee(1);
    let s = build_signer(
        &fluent_namespace(C_MAIN),
        bimap.clone(),
        &kps[OFFENDER],
        None,
    )
    .expect("member");
    let p1: Proposal<Sha256Digest> = Proposal::new(round(), View::new(VIEW - 1), digest(0xcc));
    let p2: Proposal<Sha256Digest> = Proposal::new(round(), View::new(VIEW - 1), digest(0xdd));
    let f1 = Finalize::sign(&s, p1.clone()).expect("sign f1");
    let f2 = Finalize::sign(&s, p2.clone()).expect("sign f2");
    let signer_idx = f1.attestation.signer.get();
    let s1 = sig48(
        f1.attestation
            .signature
            .get()
            .unwrap()
            .vote()
            .encode()
            .as_ref(),
    );
    let s2 = sig48(
        f2.attestation
            .signature
            .get()
            .unwrap()
            .vote()
            .encode()
            .as_ref(),
    );
    let on_chain = ConflictingFinalize::<VoteScheme, _>::new(
        Finalize {
            proposal: p1.clone(),
            attestation: vote_att(&f1.attestation),
        },
        Finalize {
            proposal: p2.clone(),
            attestation: vote_att(&f2.attestation),
        },
    );
    let evidence = hex::encode(on_chain.encode());
    let ev = ConflictingFinalize::new(f1, f2);
    let pk96: [u8; 96] = kps[OFFENDER].public_bytes();
    let vector = Vector {
        label: "conflicting_finalize",
        evidence,
        epoch: EPOCH,
        signer_idx,
        kind1: 2,
        msg1: hex::encode(p1.encode()),
        sig1: hex::encode(s1),
        kind2: 2,
        msg2: hex::encode(p2.encode()),
        sig2: hex::encode(s2),
        pk_unc: hex::encode(pubkey_compressed_to_eip2537(&pk96).unwrap()),
        sig1_unc: hex::encode(signature_compressed_to_eip2537(&s1).unwrap()),
        sig2_unc: hex::encode(signature_compressed_to_eip2537(&s2).unwrap()),
    };
    (vector, ev, bimap)
}

fn nullify_finalize() -> (
    Vector,
    NullifyFinalize<fluentbase_bls::Scheme, Sha256Digest>,
    BiMap<PeerPubkey, BlsPubkey>,
) {
    let (kps, bimap) = committee(1);
    let s = build_signer(
        &fluent_namespace(C_MAIN),
        bimap.clone(),
        &kps[OFFENDER],
        None,
    )
    .expect("member");
    let nul = commonware_consensus::simplex::types::Nullify::sign::<Sha256Digest>(&s, round())
        .expect("sign nullify");
    let prop: Proposal<Sha256Digest> = Proposal::new(round(), View::new(VIEW - 1), digest(0xee));
    let fin = Finalize::sign(&s, prop.clone()).expect("sign finalize");
    let signer_idx = nul.attestation.signer.get();
    let s1 = sig48(
        nul.attestation
            .signature
            .get()
            .unwrap()
            .vote()
            .encode()
            .as_ref(),
    );
    let s2 = sig48(
        fin.attestation
            .signature
            .get()
            .unwrap()
            .vote()
            .encode()
            .as_ref(),
    );
    let on_chain = NullifyFinalize::<VoteScheme, _>::new(
        commonware_consensus::simplex::types::Nullify {
            round: nul.round,
            attestation: vote_att(&nul.attestation),
        },
        Finalize {
            proposal: prop.clone(),
            attestation: vote_att(&fin.attestation),
        },
    );
    let evidence = hex::encode(on_chain.encode());
    let ev = NullifyFinalize::new(nul, fin);
    let pk96: [u8; 96] = kps[OFFENDER].public_bytes();
    let vector = Vector {
        label: "nullify_finalize",
        evidence,
        epoch: EPOCH,
        signer_idx,
        kind1: 1, // Nullify (message = Round.encode())
        msg1: hex::encode(round().encode()),
        sig1: hex::encode(s1),
        kind2: 2, // Finalize
        msg2: hex::encode(prop.encode()),
        sig2: hex::encode(s2),
        pk_unc: hex::encode(pubkey_compressed_to_eip2537(&pk96).unwrap()),
        sig1_unc: hex::encode(signature_compressed_to_eip2537(&s1).unwrap()),
        sig2_unc: hex::encode(signature_compressed_to_eip2537(&s2).unwrap()),
    };
    (vector, ev, bimap)
}

fn all() -> Vec<Vector> {
    vec![
        conflicting_notarize().0,
        conflicting_finalize().0,
        nullify_finalize().0,
    ]
}

// Filled verbatim from `print_corpus`; reviewed in PR; mirrored into Solidity.
const EXPECTED: &[Expected] = &[
    Expected {
        label: "conflicting_notarize",
        evidence: "072a29aa000000000000000000000000000000000000000000000000000000000000aa038aa1d24f195fc333878b14744f62a363acf0051249c949c4cc473850991aa70841eea2171a333b13de2e61fed4936305072a29bb000000000000000000000000000000000000000000000000000000000000bb03923c9abd2f0abe63eed5a2d9ac175032b2b48685c61f9e6a7c8b7419d78077821d82a3bfd41a5f10bcfcd8434444f820",
        epoch: 7,
        signer_idx: 3,
        kind1: 0,
        msg1: "072a29aa000000000000000000000000000000000000000000000000000000000000aa",
        sig1: "8aa1d24f195fc333878b14744f62a363acf0051249c949c4cc473850991aa70841eea2171a333b13de2e61fed4936305",
        kind2: 0,
        msg2: "072a29bb000000000000000000000000000000000000000000000000000000000000bb",
        sig2: "923c9abd2f0abe63eed5a2d9ac175032b2b48685c61f9e6a7c8b7419d78077821d82a3bfd41a5f10bcfcd8434444f820",
        pk_unc: "0000000000000000000000000000000006b5c608826c61fe46820427bf25729c9f23127e072ddfeaf810db77af12076cdafac00980e6d4f5c2e3347053a3493d0000000000000000000000000000000000b5825c3dbaf52a1b20571d5925e944677d67b173100ea228cb38293f6e6f8d30ed9d9823250f2d7991640f49824933000000000000000000000000000000000b70c4b3fc082f10e64460dfe75ef87099cba2e9324416e7a37a679599fb54125ab10d594c18e8315d4d257ce2c8a1630000000000000000000000000000000010df56392d11ff44f778098bcb25487e92f066af30d7cdfada087e67bf0ca9a349436b8223db8ff241d364c5898319e9",
        sig1_unc: "000000000000000000000000000000000aa1d24f195fc333878b14744f62a363acf0051249c949c4cc473850991aa70841eea2171a333b13de2e61fed49363050000000000000000000000000000000008a31fb618afd2019874641885fed0833ceaada142a5ff81cda50d437e14ba2c1e10e069b2de463d691d99d9c2168cf7",
        sig2_unc: "00000000000000000000000000000000123c9abd2f0abe63eed5a2d9ac175032b2b48685c61f9e6a7c8b7419d78077821d82a3bfd41a5f10bcfcd8434444f8200000000000000000000000000000000007d0660c235f4285a0552e8cd8820deeb134e43254f4e7ffc788a62d8bfbe759649fb840f1f2529ffdaf74c54a496df2",
    },
    Expected {
        label: "conflicting_finalize",
        evidence: "072a29cc000000000000000000000000000000000000000000000000000000000000cc039936ff0962301d36721c6d9e7947ec8a340bb9b5b7fcfa74ba2582918c9b3358b31c15c2a8ae372f3340e8c7706d32a6072a29dd000000000000000000000000000000000000000000000000000000000000dd03877570329a653f6cf0916cd5332247cd29a73d60a867dc1d710d5fe1bb4449b1e9393d5f9aed23bb08a2f9aed0e65af2",
        epoch: 7,
        signer_idx: 3,
        kind1: 2,
        msg1: "072a29cc000000000000000000000000000000000000000000000000000000000000cc",
        sig1: "9936ff0962301d36721c6d9e7947ec8a340bb9b5b7fcfa74ba2582918c9b3358b31c15c2a8ae372f3340e8c7706d32a6",
        kind2: 2,
        msg2: "072a29dd000000000000000000000000000000000000000000000000000000000000dd",
        sig2: "877570329a653f6cf0916cd5332247cd29a73d60a867dc1d710d5fe1bb4449b1e9393d5f9aed23bb08a2f9aed0e65af2",
        pk_unc: "0000000000000000000000000000000006b5c608826c61fe46820427bf25729c9f23127e072ddfeaf810db77af12076cdafac00980e6d4f5c2e3347053a3493d0000000000000000000000000000000000b5825c3dbaf52a1b20571d5925e944677d67b173100ea228cb38293f6e6f8d30ed9d9823250f2d7991640f49824933000000000000000000000000000000000b70c4b3fc082f10e64460dfe75ef87099cba2e9324416e7a37a679599fb54125ab10d594c18e8315d4d257ce2c8a1630000000000000000000000000000000010df56392d11ff44f778098bcb25487e92f066af30d7cdfada087e67bf0ca9a349436b8223db8ff241d364c5898319e9",
        sig1_unc: "000000000000000000000000000000001936ff0962301d36721c6d9e7947ec8a340bb9b5b7fcfa74ba2582918c9b3358b31c15c2a8ae372f3340e8c7706d32a6000000000000000000000000000000000898025f48ea072c82938e3b8748d85f0f28a48a87131bfc5422d80427873db4441111b509967bc6eb0b673aa387ce46",
        sig2_unc: "00000000000000000000000000000000077570329a653f6cf0916cd5332247cd29a73d60a867dc1d710d5fe1bb4449b1e9393d5f9aed23bb08a2f9aed0e65af200000000000000000000000000000000018862d71974a29a8af65cf3f67b6817b5be4571e5a0962c89e37cbafb9cb58c3c4c434e3f005ac10c116b316d245a63",
    },
    Expected {
        label: "nullify_finalize",
        evidence: "072a03b9d1ed34ffda9193ce95eee9ab8db558f4e923a1b58a6f80ca0bf221f7567f72d65132b103190fd5c687f7f7a6cdc3db072a29ee000000000000000000000000000000000000000000000000000000000000ee0389eae226e709054f09892935d13772a1e73b62a8bfbd24e5f1f63617c5242714166f49b52ca5d97475b81820f75a6161",
        epoch: 7,
        signer_idx: 3,
        kind1: 1,
        msg1: "072a",
        sig1: "b9d1ed34ffda9193ce95eee9ab8db558f4e923a1b58a6f80ca0bf221f7567f72d65132b103190fd5c687f7f7a6cdc3db",
        kind2: 2,
        msg2: "072a29ee000000000000000000000000000000000000000000000000000000000000ee",
        sig2: "89eae226e709054f09892935d13772a1e73b62a8bfbd24e5f1f63617c5242714166f49b52ca5d97475b81820f75a6161",
        pk_unc: "0000000000000000000000000000000006b5c608826c61fe46820427bf25729c9f23127e072ddfeaf810db77af12076cdafac00980e6d4f5c2e3347053a3493d0000000000000000000000000000000000b5825c3dbaf52a1b20571d5925e944677d67b173100ea228cb38293f6e6f8d30ed9d9823250f2d7991640f49824933000000000000000000000000000000000b70c4b3fc082f10e64460dfe75ef87099cba2e9324416e7a37a679599fb54125ab10d594c18e8315d4d257ce2c8a1630000000000000000000000000000000010df56392d11ff44f778098bcb25487e92f066af30d7cdfada087e67bf0ca9a349436b8223db8ff241d364c5898319e9",
        sig1_unc: "0000000000000000000000000000000019d1ed34ffda9193ce95eee9ab8db558f4e923a1b58a6f80ca0bf221f7567f72d65132b103190fd5c687f7f7a6cdc3db000000000000000000000000000000000d1b14ec4b4646e2c2f0a8ada516d162b402e9368c4b00d73a8132c3574cf965ab155be8e1d3b73089380ee371fc317d",
        sig2_unc: "0000000000000000000000000000000009eae226e709054f09892935d13772a1e73b62a8bfbd24e5f1f63617c5242714166f49b52ca5d97475b81820f75a6161000000000000000000000000000000000cfe5bc40869ff0b82cdcfed30c791be592b613d057b158b338285feafae5aab51ea9df43427006bdb71a4b60c78d5c1",
    },
];

fn assert_row(v: &Vector, e: &Expected) {
    // A drift anywhere here means the Simplex consensus wire format
    // (Commonware codec) changed:
    // regenerate via `--ignored print_corpus`, review the diff, and mirror
    // into the Solidity test in the same PR.
    assert_eq!(v.label, e.label, "label drift");
    assert_eq!(v.evidence, e.evidence, "{} evidence drift", v.label);
    assert_eq!(v.epoch, e.epoch, "{} epoch drift", v.label);
    assert_eq!(v.signer_idx, e.signer_idx, "{} signer_idx drift", v.label);
    assert_eq!(v.kind1, e.kind1, "{} kind1 drift", v.label);
    assert_eq!(v.msg1, e.msg1, "{} msg1 drift", v.label);
    assert_eq!(v.sig1, e.sig1, "{} sig1 drift", v.label);
    assert_eq!(v.kind2, e.kind2, "{} kind2 drift", v.label);
    assert_eq!(v.msg2, e.msg2, "{} msg2 drift", v.label);
    assert_eq!(v.sig2, e.sig2, "{} sig2 drift", v.label);
    assert_eq!(v.pk_unc, e.pk_unc, "{} pk_unc drift", v.label);
    assert_eq!(v.sig1_unc, e.sig1_unc, "{} sig1_unc drift", v.label);
    assert_eq!(v.sig2_unc, e.sig2_unc, "{} sig2_unc drift", v.label);
}

#[test]
fn conformance_corpus_matches_committed_constants() {
    let got = all();
    assert_eq!(
        got.len(),
        EXPECTED.len(),
        "EXPECTED and recipes out of sync"
    );
    for (v, e) in got.iter().zip(EXPECTED.iter()) {
        assert_row(v, e);
    }
    let cm = committee_dump();
    assert_eq!(cm.len(), COMMITTEE.len(), "COMMITTEE out of sync");
    for (i, ((p, b), e)) in cm.iter().zip(COMMITTEE.iter()).enumerate() {
        assert_eq!(
            (p.as_str(), b.as_str()),
            (e.0, e.1),
            "committee drift at {i}"
        );
    }
    let pop = committee_pop_dump();
    assert_eq!(pop.len(), COMMITTEE_POP.len(), "COMMITTEE_POP out of sync");
    for (i, ((s, su, pu), e)) in pop.iter().zip(COMMITTEE_POP.iter()).enumerate() {
        assert_eq!(
            (s.as_str(), su.as_str(), pu.as_str()),
            (e.0, e.1, e.2),
            "committee pop drift at {i}"
        );
    }
}

#[test]
fn helper_extract_args_matches_pinned_corpus() {
    use fluentbase_consensus::slasher::evidence::{
        extract_from_conflicting_finalize, extract_from_conflicting_notarize,
        extract_from_nullify_finalize, SlashKind,
    };

    let (v_cn, cn, bimap_cn) = conflicting_notarize();
    let committee_cn = fluentbase_bls::EpochCommittee::from_unverified(EPOCH, bimap_cn);
    let args_cn = extract_from_conflicting_notarize(&cn, &committee_cn)
        .expect("extract conflicting_notarize");
    assert_eq!(
        hex::encode(&args_cn.evidence),
        v_cn.evidence,
        "evidence drift (conflicting_notarize)"
    );
    assert_eq!(
        hex::encode(args_cn.pk_uncompressed),
        v_cn.pk_unc,
        "pk_unc drift"
    );
    assert_eq!(
        hex::encode(args_cn.sig1_uncompressed),
        v_cn.sig1_unc,
        "sig1_unc drift"
    );
    assert_eq!(
        hex::encode(args_cn.sig2_uncompressed),
        v_cn.sig2_unc,
        "sig2_unc drift"
    );
    assert_eq!(args_cn.kind, SlashKind::ConflictingNotarize, "kind drift");

    let (v_cf, cf, bimap_cf) = conflicting_finalize();
    let committee_cf = fluentbase_bls::EpochCommittee::from_unverified(EPOCH, bimap_cf);
    let args_cf = extract_from_conflicting_finalize(&cf, &committee_cf)
        .expect("extract conflicting_finalize");
    assert_eq!(hex::encode(&args_cf.evidence), v_cf.evidence);
    assert_eq!(hex::encode(args_cf.pk_uncompressed), v_cf.pk_unc);
    assert_eq!(hex::encode(args_cf.sig1_uncompressed), v_cf.sig1_unc);
    assert_eq!(hex::encode(args_cf.sig2_uncompressed), v_cf.sig2_unc);
    assert_eq!(args_cf.kind, SlashKind::ConflictingFinalize);

    let (v_nf, nf, bimap_nf) = nullify_finalize();
    let committee_nf = fluentbase_bls::EpochCommittee::from_unverified(EPOCH, bimap_nf);
    let args_nf =
        extract_from_nullify_finalize(&nf, &committee_nf).expect("extract nullify_finalize");
    assert_eq!(hex::encode(&args_nf.evidence), v_nf.evidence);
    assert_eq!(hex::encode(args_nf.pk_uncompressed), v_nf.pk_unc);
    assert_eq!(hex::encode(args_nf.sig1_uncompressed), v_nf.sig1_unc);
    assert_eq!(hex::encode(args_nf.sig2_uncompressed), v_nf.sig2_unc);
    assert_eq!(args_nf.kind, SlashKind::NullifyFinalize);
}

/// Full-pin extension: extract → ABI-encode → compare against an in-test
/// reference encoding computed via the canonical `SolCall` macro (selector +
/// `abi.encode(...)`). Drift here means a Solidity function signature change,
/// a `SlashCallArgs` field-order change, or an `alloy-sol-types` ABI codec
/// change — any of which break the `equivocation_slashing` on-chain path.
#[test]
fn helper_extract_then_abi_encode_matches_pinned_calldata() {
    use alloy_primitives::Bytes as AlloyBytes;
    use alloy_sol_types::{sol, SolCall};
    use fluentbase_consensus::slasher::evidence::{
        extract_from_conflicting_finalize, extract_from_conflicting_notarize,
        extract_from_nullify_finalize, SlashCallArgs, SlashKind,
    };

    // Mirror of the bindings in `crates/consensus/src/slasher/actor.rs` — pinned
    // here so a future Solidity signature change is caught at the conformance
    // boundary (the slasher is the only on-chain caller, but the contract ABI
    // must move in lockstep with `SlashCallArgs`).
    sol! {
        function slashEquivocationNotarize(bytes evidence, bytes pkUncompressed,
            bytes sig1Uncompressed, bytes sig2Uncompressed) external;
        function slashEquivocationFinalize(bytes evidence, bytes pkUncompressed,
            bytes sig1Uncompressed, bytes sig2Uncompressed) external;
        function slashEquivocationNullifyFinalize(bytes evidence, bytes pkUncompressed,
            bytes sig1Uncompressed, bytes sig2Uncompressed) external;
    }

    fn encode(args: &SlashCallArgs) -> Vec<u8> {
        let evidence = AlloyBytes::from(args.evidence.clone());
        let pk = AlloyBytes::from(args.pk_uncompressed.to_vec());
        let s1 = AlloyBytes::from(args.sig1_uncompressed.to_vec());
        let s2 = AlloyBytes::from(args.sig2_uncompressed.to_vec());
        match args.kind {
            SlashKind::ConflictingNotarize => slashEquivocationNotarizeCall {
                evidence,
                pkUncompressed: pk,
                sig1Uncompressed: s1,
                sig2Uncompressed: s2,
            }
            .abi_encode(),
            SlashKind::ConflictingFinalize => slashEquivocationFinalizeCall {
                evidence,
                pkUncompressed: pk,
                sig1Uncompressed: s1,
                sig2Uncompressed: s2,
            }
            .abi_encode(),
            SlashKind::NullifyFinalize => slashEquivocationNullifyFinalizeCall {
                evidence,
                pkUncompressed: pk,
                sig1Uncompressed: s1,
                sig2Uncompressed: s2,
            }
            .abi_encode(),
        }
    }

    let (_v_cn, cn, bimap_cn) = conflicting_notarize();
    let committee_cn = fluentbase_bls::EpochCommittee::from_unverified(EPOCH, bimap_cn);
    let args_cn = extract_from_conflicting_notarize(&cn, &committee_cn).unwrap();
    let calldata_cn = encode(&args_cn);
    assert_eq!(
        &calldata_cn[..4],
        &slashEquivocationNotarizeCall::SELECTOR,
        "conflicting_notarize selector drift",
    );
    let decoded_cn = slashEquivocationNotarizeCall::abi_decode(&calldata_cn).expect("decode cn");
    assert_eq!(decoded_cn.evidence.as_ref(), &args_cn.evidence[..]);
    assert_eq!(
        decoded_cn.pkUncompressed.as_ref(),
        &args_cn.pk_uncompressed[..]
    );
    assert_eq!(
        decoded_cn.sig1Uncompressed.as_ref(),
        &args_cn.sig1_uncompressed[..]
    );
    assert_eq!(
        decoded_cn.sig2Uncompressed.as_ref(),
        &args_cn.sig2_uncompressed[..]
    );

    let (_v_cf, cf, bimap_cf) = conflicting_finalize();
    let committee_cf = fluentbase_bls::EpochCommittee::from_unverified(EPOCH, bimap_cf);
    let args_cf = extract_from_conflicting_finalize(&cf, &committee_cf).unwrap();
    let calldata_cf = encode(&args_cf);
    assert_eq!(
        &calldata_cf[..4],
        &slashEquivocationFinalizeCall::SELECTOR,
        "conflicting_finalize selector drift",
    );
    let decoded_cf = slashEquivocationFinalizeCall::abi_decode(&calldata_cf).expect("decode cf");
    assert_eq!(decoded_cf.evidence.as_ref(), &args_cf.evidence[..]);
    assert_eq!(
        decoded_cf.pkUncompressed.as_ref(),
        &args_cf.pk_uncompressed[..]
    );
    assert_eq!(
        decoded_cf.sig1Uncompressed.as_ref(),
        &args_cf.sig1_uncompressed[..]
    );
    assert_eq!(
        decoded_cf.sig2Uncompressed.as_ref(),
        &args_cf.sig2_uncompressed[..]
    );

    let (_v_nf, nf, bimap_nf) = nullify_finalize();
    let committee_nf = fluentbase_bls::EpochCommittee::from_unverified(EPOCH, bimap_nf);
    let args_nf = extract_from_nullify_finalize(&nf, &committee_nf).unwrap();
    let calldata_nf = encode(&args_nf);
    assert_eq!(
        &calldata_nf[..4],
        &slashEquivocationNullifyFinalizeCall::SELECTOR,
        "nullify_finalize selector drift",
    );
    let decoded_nf =
        slashEquivocationNullifyFinalizeCall::abi_decode(&calldata_nf).expect("decode nf");
    assert_eq!(decoded_nf.evidence.as_ref(), &args_nf.evidence[..]);
    assert_eq!(
        decoded_nf.pkUncompressed.as_ref(),
        &args_nf.pk_uncompressed[..]
    );
    assert_eq!(
        decoded_nf.sig1Uncompressed.as_ref(),
        &args_nf.sig1_uncompressed[..]
    );
    assert_eq!(
        decoded_nf.sig2Uncompressed.as_ref(),
        &args_nf.sig2_uncompressed[..]
    );

    // All three selectors must be distinct (a name collision would route
    // to the wrong Solidity branch).
    assert_ne!(
        &slashEquivocationNotarizeCall::SELECTOR,
        &slashEquivocationFinalizeCall::SELECTOR,
    );
    assert_ne!(
        &slashEquivocationFinalizeCall::SELECTOR,
        &slashEquivocationNullifyFinalizeCall::SELECTOR,
    );
    assert_ne!(
        &slashEquivocationNotarizeCall::SELECTOR,
        &slashEquivocationNullifyFinalizeCall::SELECTOR,
    );
}

#[test]
#[ignore = "regenerator: prints the pinned corpus for hand-mirroring"]
fn print_corpus() {
    println!("\nconst EXPECTED: &[Expected] = &[");
    for v in all() {
        println!("    Expected {{");
        println!("        label: {:?},", v.label);
        println!("        evidence: {:?},", v.evidence);
        println!("        epoch: {},", v.epoch);
        println!("        signer_idx: {},", v.signer_idx);
        println!("        kind1: {},", v.kind1);
        println!("        msg1: {:?},", v.msg1);
        println!("        sig1: {:?},", v.sig1);
        println!("        kind2: {},", v.kind2);
        println!("        msg2: {:?},", v.msg2);
        println!("        sig2: {:?},", v.sig2);
        println!("        pk_unc: {:?},", v.pk_unc);
        println!("        sig1_unc: {:?},", v.sig1_unc);
        println!("        sig2_unc: {:?},", v.sig2_unc);
        println!("    }},");
    }
    println!("];");
    println!("\nconst COMMITTEE: &[(&str, &str)] = &[");
    for (p, b) in committee_dump() {
        println!("    ({p:?}, {b:?}),");
    }
    println!("]; // OFFENDER = {OFFENDER}");
    println!("\nconst COMMITTEE_POP: &[(&str, &str, &str)] = &[");
    for (s, su, pu) in committee_pop_dump() {
        println!("    ({s:?}, {su:?}, {pu:?}),");
    }
    println!("];");
}
