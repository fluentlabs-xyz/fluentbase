//! Cross-language BLS conformance corpus (PoP, MinSig) — Rust side.
//!
//! These constants are the SINGLE SOURCE mirrored by-hand into
//! `solidity-contracts` `test/bls/Eip2537ConformanceVectors.sol`
//! (see `crates/bls/CONFORMANCE.md`). The test below recomputes every
//! value from its deterministic recipe through the shipped public API and
//! asserts equality — so a Commonware/blst wire-format change makes this
//! test FAIL LOUDLY (it is the drift detector; no generator binary exists).
//!
//! Regenerate after a deliberate dependency bump:
//!   cargo test -p fluentbase-bls --test eip2537_conformance_vectors \
//!       -- --ignored print_corpus --nocapture

mod common;

use common::{pubkey_eip2537_to_compressed, signature_eip2537_to_compressed};
use commonware_codec::Encode;
use commonware_cryptography::bls12381::primitives::{
    ops,
    variant::{MinSig, Variant},
};
use commonware_math::algebra::CryptoGroup;
use fluentbase_bls::{
    encoding::{pubkey_compressed_to_eip2537, signature_compressed_to_eip2537},
    fluent_namespace,
    keys::ValidatorBlsKeypair,
    pop::{sign_pop, verify_pop},
};
use rand_08::rngs::StdRng;
use rand_core::SeedableRng;

// G2 (pubkey) lives in commonware's group module; reached via the Variant assoc type.
type G2 = <MinSig as Variant>::Public;

const OFF: u64 = 1_000_000;
const C_MAIN: u64 = 20_994;

fn key(seed: u64) -> ValidatorBlsKeypair {
    ValidatorBlsKeypair::generate(&mut StdRng::seed_from_u64(seed))
}

/// `hm` = the authoritative PoP hash-to-curve, identical to what
/// `ops::sign/verify_proof_of_possession` compute internally.
fn hm_eip2537(seed: u64, chain_id: u64) -> [u8; 128] {
    let k = key(seed);
    let ns = fluent_namespace(chain_id);
    let hm: <MinSig as Variant>::Signature =
        ops::hash_with_namespace::<MinSig>(MinSig::PROOF_OF_POSSESSION, &ns, &k.public_bytes());
    let comp: [u8; 48] = hm.encode().as_ref().try_into().expect("G1 hm is 48 B");
    signature_compressed_to_eip2537(&comp).expect("hm is a valid G1 point")
}

fn pubkey_eip2537(seed: u64) -> [u8; 256] {
    pubkey_compressed_to_eip2537(&key(seed).public_bytes()).expect("valid G2")
}

fn sig_eip2537(seed: u64, chain_id: u64) -> [u8; 128] {
    let s = sign_pop(&key(seed), &fluent_namespace(chain_id));
    signature_compressed_to_eip2537(&s).expect("valid G1")
}

fn neg_g2_generator_eip2537() -> [u8; 256] {
    let neg = -G2::generator();
    let comp: [u8; 96] = neg.encode().as_ref().try_into().expect("G2 is 96 B");
    pubkey_compressed_to_eip2537(&comp).expect("valid G2")
}

#[derive(Clone, Copy)]
enum Kind {
    Valid,
    TamperedSig,
    TamperedPubkey,
    TamperedNamespace,
}

struct Recipe {
    label: &'static str,
    seed: u64,
    chain_id: u64,
    kind: Kind,
}

// Lean set: every entry is a distinct test dimension. 4 valid (2 keys on
// main chain + the two chain_id byte-boundaries) + 3 negatives (one per
// tamper kind). No repeated "another random valid key" / "negative ×
// boundary" combos — those added no logical coverage.
const RECIPES: &[Recipe] = &[
    Recipe { label: "pop_valid_seed0", seed: 0, chain_id: C_MAIN, kind: Kind::Valid },
    Recipe { label: "pop_valid_seed1", seed: 1, chain_id: C_MAIN, kind: Kind::Valid },
    Recipe { label: "pop_valid_chain0", seed: 10, chain_id: 0, kind: Kind::Valid },
    Recipe { label: "pop_valid_chain_max", seed: 11, chain_id: u64::MAX, kind: Kind::Valid },
    Recipe { label: "pop_tampered_sig", seed: 0, chain_id: C_MAIN, kind: Kind::TamperedSig },
    Recipe { label: "pop_tampered_pubkey", seed: 0, chain_id: C_MAIN, kind: Kind::TamperedPubkey },
    Recipe {
        label: "pop_tampered_namespace",
        seed: 0,
        chain_id: C_MAIN,
        kind: Kind::TamperedNamespace,
    },
];

/// Recompute the (pubkey_eip2537, sig_eip2537, hm_eip2537, expected_valid) for a recipe.
fn build(r: &Recipe) -> ([u8; 256], [u8; 128], [u8; 128], bool) {
    match r.kind {
        Kind::Valid => (
            pubkey_eip2537(r.seed),
            sig_eip2537(r.seed, r.chain_id),
            hm_eip2537(r.seed, r.chain_id),
            true,
        ),
        Kind::TamperedSig => (
            pubkey_eip2537(r.seed),
            sig_eip2537(r.seed + OFF, r.chain_id), // valid but wrong key's sig
            hm_eip2537(r.seed, r.chain_id),
            false,
        ),
        Kind::TamperedPubkey => (
            pubkey_eip2537(r.seed + OFF), // valid but wrong key's pubkey
            sig_eip2537(r.seed, r.chain_id),
            hm_eip2537(r.seed, r.chain_id),
            false,
        ),
        Kind::TamperedNamespace => (
            pubkey_eip2537(r.seed),
            sig_eip2537(r.seed, r.chain_id),
            hm_eip2537(r.seed, r.chain_id + 1), // hm under a different namespace
            false,
        ),
    }
}

// COMMITTED CONSTANTS — single source of truth, hand-mirrored into Solidity.
// Lowercase, no 0x prefix (so they paste directly into Solidity `hex"..."`).
// (Filled by running the `print_corpus` ignored test once.)

const NEG_G2_GENERATOR_EIP2537: &str = "00000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb80000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000d1b3cc2c7027888be51d9ef691d77bcb679afda66c73f17f9ee3837a55024f78c71363275a75d75d86bab79f74782aa0000000000000000000000000000000013fa4d4a0ad8b1ce186ed5061789213d993923066dddaf1040bc3ff59f825c78df74f2d75467e25e0f55f8a00fa030ed";

/// (label, pubkey_eip2537, sig_eip2537, hm_eip2537, expected_valid)
const EXPECTED: &[(&str, &str, &str, &str, bool)] = &[
    ("pop_valid_seed0", "000000000000000000000000000000000727ef1c60e48042142f7bcc8b6382305cd50c5a4542c44ec72a4de6640c194f8ef36bea1dbed168ab6fd8681d910d550000000000000000000000000000000012b050b6fbe80695b5d56835e978918e37c8707a7fad09a01ae782d4c3170c9baa4c2c196b36eac6b78ceb210b287aeb000000000000000000000000000000000f9da5ef5089f62dc55ec91c2459f6ed3fd9981f8d4926ad90dca0314603ae4af86c8fa12bdd2569867f05a24908b7fc0000000000000000000000000000000009ac1ba2c6341d99ba0d6bfab8ea6a3a58726e787ab22b899cd95acfec350c1fc09f5fcbbef992106b61e45eb9158354", "00000000000000000000000000000000027ecd57f1889127d81b2a3c46e1905c419302192ebc90f818c7d272b38a6495337f7dde0733d0d431fc1338e8caf62e00000000000000000000000000000000109a4722abb94b2ffb8685abe75b4fc8336d2f6534b64fee49baa07ab7357de65036fb93ee119860768cc65daa4c7b1e", "0000000000000000000000000000000012a9663d3344e548e9e5904937c509f34f4010dec0524acbfef9153c3a27565cfc8d566d66973ac6a5b56e9cbd1add1b00000000000000000000000000000000003fc116448fc5b130d27f7ba81af3eb0df61a51c954fd1487efc15101ec9ef4ed0b38716a8dd09fcb9bc2daf28093ec", true),
    ("pop_valid_seed1", "00000000000000000000000000000000055a6834140fb33de0b054a7bdd63833d40e12bbdf9828cccc21de1ac819edd4d55bcfd1e0b893793cfb51afedb63bb90000000000000000000000000000000005bcdb782d3c7c11e637d6cc2609ecf1caba021c5c8f7db49d70fb726f863638d2cc9e4ea135776b9e2a32075990781100000000000000000000000000000000089b9c0041057c2ef9057b78f4d0b940b1fb72454208034d779e9a69963272655d260eb3140a410d40493f55782a2282000000000000000000000000000000000a3b4c822a3d6c328c60b7d7171a6ba1989d4722519b07fb5dd971e88bad0f94e5e08efb728e78bd4d074cabe8bb348a", "00000000000000000000000000000000112ec227d2df5a33eb496e517812442b815b97956e8146a29d2afdb87963471548628f0286d9a2dca5e7e941d99f20b8000000000000000000000000000000001871d0ad00641626a759c231910dd273ab4698e5a594061bae0b25c0b3dbccb7a6e0ee890906259e1c7399ea51da5a9c", "00000000000000000000000000000000151561b1f0660c42f56a8eeaed6e28c2e832c1a55ffdae4ab61ae29fe9da5eee05443dd445e744733bc37781388f42ce00000000000000000000000000000000094404d43c3165ee1d454aa7dee00fc49a1d4641e9a720468e6974e0ba3efa895c391d89c0cde32f8672667809b3b515", true),
    ("pop_valid_chain0", "0000000000000000000000000000000002de1aa3751856d48f0c4e45a99ccb135cb91782585162f6eecce0ca4e7f5ffbb2c33ce40a6521b8f262f8d1fbf7135d00000000000000000000000000000000037ddff9b89cf180b72e036147f79d011a5dd5ba549c196be894a04a65dc7f8f4c99639e13606c2eeeb56875d073ea45000000000000000000000000000000001051f8cdb9d3d3ac9fe6c486abe6d388258055447f42f62718383c9a41d0a53cf00f576478771740d331f136737e6e9b0000000000000000000000000000000002cc299a65a068f27d733031659ddff6e2bc74b90024f13110403a12cb9da3bea8f8b56768dc04dd08190bffaa7bbb2d", "000000000000000000000000000000000f66169165d0cb4a1ce81d50946bb44d26f8948ea17388ec5f8522a82752c22916bedb4b0371b47694a6a4a0ab12853a0000000000000000000000000000000009169884bf9deb2fa2234d45eadbe876a357aa7b53153ecf2a95db003e572ffa0ce4852c582f4959911b7e2ac3a3f730", "00000000000000000000000000000000177229c119e979f84d25b8c73fdaa0aa26317416b1f38216f9f9315d2359f5aab938c5e9a5e8f5f6f5e0c5a9a334922c000000000000000000000000000000000d2eb923c274599b712f303357d64851d61627e28785ddd39c53815a53b9e0d9b944fa49eead3ecc5058237fa775ce18", true),
    ("pop_valid_chain_max", "000000000000000000000000000000000dfa64ce660e6119ee663993062f13caefe8576ff2e7599607137a884bcaef7c7de9165609ba1e729fc98e7d209863820000000000000000000000000000000018cdf5244674d9e9322a0e468082645c90e400607a4c82fc418a42ca85edb2b8cd98d4afc18bdc1f8017e00b7344c8e80000000000000000000000000000000011d2b3f72e5bcd1d53a5d9b55fded9e366ea1acbdd7e0912ab78fa134923416993e8f7f552627c88fbed8c8117f840ca0000000000000000000000000000000015cffd54afeca1f2d6e93c77c58f7e08c967269628183fe00722b06c0d8ccc5d42cb4c5a6f175a6b0e2b1bf353c8bccb", "00000000000000000000000000000000039fce145e9ede7e3f2e648e4cfe4eedaca9da3caab0d16351377e50f7eeba6e16765cdac6c18f115639c8a602e59c74000000000000000000000000000000000d043bf449d73052aa672d5202841729d393285e2d8ec0afba025d18507015c4fe9d0f3adfe63e199427d4a9a9745469", "0000000000000000000000000000000009375dff316e8c8b11a75f0cfb671552d19d7facf47d931f821dbfcee40fb59389cef33906cf93a9b326d285974056e30000000000000000000000000000000000bfd0963d1218708e96e99366d4d41b5a9354ad33a7086454e69cbd7868d748a834f7b4e16fdbecf824a4e0ec6a1253", true),
    ("pop_tampered_sig", "000000000000000000000000000000000727ef1c60e48042142f7bcc8b6382305cd50c5a4542c44ec72a4de6640c194f8ef36bea1dbed168ab6fd8681d910d550000000000000000000000000000000012b050b6fbe80695b5d56835e978918e37c8707a7fad09a01ae782d4c3170c9baa4c2c196b36eac6b78ceb210b287aeb000000000000000000000000000000000f9da5ef5089f62dc55ec91c2459f6ed3fd9981f8d4926ad90dca0314603ae4af86c8fa12bdd2569867f05a24908b7fc0000000000000000000000000000000009ac1ba2c6341d99ba0d6bfab8ea6a3a58726e787ab22b899cd95acfec350c1fc09f5fcbbef992106b61e45eb9158354", "000000000000000000000000000000001733f7c8769099b3c5f2601d80aec5f35b4e0086b9d4f2092140e0f40002c328ceb71b469d9456ed4caa27e340a78d9b00000000000000000000000000000000058d6642e4126b5d37407dcb4a34911ceab3992b1524ce67bf5fdd2688374a692839dba9ac3ba6ab3305cf51200d49ca", "0000000000000000000000000000000012a9663d3344e548e9e5904937c509f34f4010dec0524acbfef9153c3a27565cfc8d566d66973ac6a5b56e9cbd1add1b00000000000000000000000000000000003fc116448fc5b130d27f7ba81af3eb0df61a51c954fd1487efc15101ec9ef4ed0b38716a8dd09fcb9bc2daf28093ec", false),
    ("pop_tampered_pubkey", "000000000000000000000000000000001786fbbbd8023cda9bcc54aca136b008c221f1affb6edf649e997b88608ad54e5e8808e140fca75a6d9201ab1391544d00000000000000000000000000000000156cc05952690c6fefd4e845b776344f4f87823af39e5e05edf03d0249ce25f89e358b35d5f735cb2f4c541b86243cf60000000000000000000000000000000016419ccb7181a718690801bf7d6d9deaf1aae6ad3ca04bd8757e592fcf3b51c6e2058b9f09c47900074229296713c5f8000000000000000000000000000000000dc5b946b1e0f39d0a2eb13fc80903aff14265a6d2e8ff70571e7accc99847f803518e36f1c68c55cb06f3a36315d342", "00000000000000000000000000000000027ecd57f1889127d81b2a3c46e1905c419302192ebc90f818c7d272b38a6495337f7dde0733d0d431fc1338e8caf62e00000000000000000000000000000000109a4722abb94b2ffb8685abe75b4fc8336d2f6534b64fee49baa07ab7357de65036fb93ee119860768cc65daa4c7b1e", "0000000000000000000000000000000012a9663d3344e548e9e5904937c509f34f4010dec0524acbfef9153c3a27565cfc8d566d66973ac6a5b56e9cbd1add1b00000000000000000000000000000000003fc116448fc5b130d27f7ba81af3eb0df61a51c954fd1487efc15101ec9ef4ed0b38716a8dd09fcb9bc2daf28093ec", false),
    ("pop_tampered_namespace", "000000000000000000000000000000000727ef1c60e48042142f7bcc8b6382305cd50c5a4542c44ec72a4de6640c194f8ef36bea1dbed168ab6fd8681d910d550000000000000000000000000000000012b050b6fbe80695b5d56835e978918e37c8707a7fad09a01ae782d4c3170c9baa4c2c196b36eac6b78ceb210b287aeb000000000000000000000000000000000f9da5ef5089f62dc55ec91c2459f6ed3fd9981f8d4926ad90dca0314603ae4af86c8fa12bdd2569867f05a24908b7fc0000000000000000000000000000000009ac1ba2c6341d99ba0d6bfab8ea6a3a58726e787ab22b899cd95acfec350c1fc09f5fcbbef992106b61e45eb9158354", "00000000000000000000000000000000027ecd57f1889127d81b2a3c46e1905c419302192ebc90f818c7d272b38a6495337f7dde0733d0d431fc1338e8caf62e00000000000000000000000000000000109a4722abb94b2ffb8685abe75b4fc8336d2f6534b64fee49baa07ab7357de65036fb93ee119860768cc65daa4c7b1e", "000000000000000000000000000000000f35aac942358f6b42a7dbdd8a01f5ed2c905955bf2920314c7f8a7edf61cb8787a1e3ecfcbc2ee886e2e7dc8ea7c6e10000000000000000000000000000000001987550b6fc24d90c7298b7e35c0724345e9ded0df6e4e28ca9ced34d6a4b62b2d7e251070d80e97d64763e3adeeffd", false),
];

fn dehex<const N: usize>(s: &str) -> [u8; N] {
    let v = hex::decode(s).expect("valid hex");
    v.as_slice().try_into().expect("hex length matches N")
}

#[test]
fn conformance_corpus_matches_committed_constants() {
    assert_eq!(
        EXPECTED.len(),
        RECIPES.len(),
        "EXPECTED table and RECIPES out of sync"
    );
    assert_eq!(
        neg_g2_generator_eip2537(),
        dehex::<256>(NEG_G2_GENERATOR_EIP2537),
        "NEG_G2_GENERATOR drifted — Commonware/blst wire format changed; \
         regenerate via `--ignored print_corpus`, review the diff, update \
         this file AND the Solidity mirror in the same PR"
    );
    for (r, e) in RECIPES.iter().zip(EXPECTED.iter()) {
        assert_eq!(r.label, e.0, "recipe/expected label mismatch");
        let (pk, sig, hm, valid) = build(r);
        assert_eq!(pk, dehex::<256>(e.1), "pubkey drift: {}", r.label);
        assert_eq!(sig, dehex::<128>(e.2), "sig drift: {}", r.label);
        assert_eq!(hm, dehex::<128>(e.3), "hm drift: {}", r.label);
        assert_eq!(valid, e.4, "expected_valid mismatch: {}", r.label);

        // The TamperedNamespace recipe signs under ns(chain_id) but encodes
        // `hm` under ns(chain_id + 1); to make verify_pop diverge equally,
        // verify against the same tampered namespace.
        let ns = match r.kind {
            Kind::TamperedNamespace => fluent_namespace(r.chain_id + 1),
            _ => fluent_namespace(r.chain_id),
        };
        let pk_comp = pubkey_eip2537_to_compressed(&pk).expect("pinned pk must convert");
        let sig_comp = signature_eip2537_to_compressed(&sig).expect("pinned sig must convert");
        let verify_ok = verify_pop(&pk_comp, &ns, &sig_comp).is_ok();
        assert_eq!(
            verify_ok, valid,
            "verify_pop disagrees with recipe `{}`: got={}, expected={}",
            r.label, verify_ok, valid,
        );
    }
}

/// Not a test — the regeneration tool. Prints the corpus as hex to paste
/// into EXPECTED above, into `crates/bls/CONFORMANCE.md`, and into the
/// Solidity mirror. Run with `-- --ignored print_corpus --nocapture`.
#[test]
#[ignore]
fn print_corpus() {
    println!(
        "NEG_G2_GENERATOR_EIP2537 = \"{}\"",
        hex::encode(neg_g2_generator_eip2537())
    );
    for r in RECIPES {
        let (pk, sig, hm, valid) = build(r);
        println!(
            "(\"{}\", \"{}\", \"{}\", \"{}\", {})",
            r.label,
            hex::encode(pk),
            hex::encode(sig),
            hex::encode(hm),
            valid
        );
    }
}
