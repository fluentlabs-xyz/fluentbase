//! Cross-language hash-to-G1 conformance pin (RFC 9380, MinSig, both DSTs): the
//! on-chain `hash_to_g1` (`contracts/staking/src/bls.rs`) must produce the same G1
//! point as `commonware ops::hash_with_namespace::<MinSig>` for both
//! domain-separation tags (`PROOF_OF_POSSESSION` and `MESSAGE`), or signatures the
//! node accepts fail verification on-chain.
//!
//! `EXPECTED_H` is mirrored by hand into
//! `solidity-contracts/test/bls/BlsHashToG1Conformance.t.sol`; update both together.

use commonware_codec::Encode;
use commonware_cryptography::bls12381::primitives::{
    ops,
    variant::{MinSig, Variant},
};
use fluentbase_bls::{
    encoding::{pubkey_compressed_to_eip2537, signature_compressed_to_eip2537},
    fluent_namespace,
    keys::ValidatorBlsKeypair,
    pop::sign_pop,
};
use rand_08::rngs::StdRng;
use rand_core::{RngCore, SeedableRng};

// Simplex per-subject namespace suffixes; must match the consensus scheme's.
const NOTARIZE_SUFFIX: &[u8] = b"_NOTARIZE";
const NULLIFY_SUFFIX: &[u8] = b"_NULLIFY";
const FINALIZE_SUFFIX: &[u8] = b"_FINALIZE";

const C_MAIN: u64 = 20_994;

#[derive(Clone, Copy)]
enum Subject {
    Pop,
    Notarize,
    Nullify,
    Finalize,
}

struct Recipe {
    label: &'static str,
    chain_id: u64,
    subject: Subject,
    msg_seed: u64,
    msg_len: usize,
}

const RECIPES: &[Recipe] = &[
    Recipe {
        label: "pop_main_pk96",
        chain_id: C_MAIN,
        subject: Subject::Pop,
        msg_seed: 0,
        msg_len: 96,
    },
    Recipe {
        label: "pop_chain0_empty",
        chain_id: 0,
        subject: Subject::Pop,
        msg_seed: 1,
        msg_len: 0,
    },
    Recipe {
        label: "notarize_main_proposal",
        chain_id: C_MAIN,
        subject: Subject::Notarize,
        msg_seed: 2,
        msg_len: 80,
    },
    Recipe {
        label: "nullify_main_round",
        chain_id: C_MAIN,
        subject: Subject::Nullify,
        msg_seed: 3,
        msg_len: 11,
    },
    Recipe {
        label: "finalize_chainmax_long",
        chain_id: u64::MAX,
        subject: Subject::Finalize,
        msg_seed: 4,
        msg_len: 200,
    },
    Recipe {
        label: "sig_chain0_short",
        chain_id: 0,
        subject: Subject::Notarize,
        msg_seed: 5,
        msg_len: 1,
    },
];

fn ns_of(r: &Recipe) -> Vec<u8> {
    let mut ns = fluent_namespace(r.chain_id);
    match r.subject {
        Subject::Pop => {}
        Subject::Notarize => ns.extend_from_slice(NOTARIZE_SUFFIX),
        Subject::Nullify => ns.extend_from_slice(NULLIFY_SUFFIX),
        Subject::Finalize => ns.extend_from_slice(FINALIZE_SUFFIX),
    }
    ns
}

fn msg_of(r: &Recipe) -> Vec<u8> {
    let mut m = vec![0u8; r.msg_len];
    StdRng::seed_from_u64(r.msg_seed).fill_bytes(&mut m);
    m
}

fn dst_of(r: &Recipe) -> &'static [u8] {
    match r.subject {
        Subject::Pop => MinSig::PROOF_OF_POSSESSION,
        _ => MinSig::MESSAGE,
    }
}

fn hash_eip2537(r: &Recipe) -> [u8; 128] {
    let ns = ns_of(r);
    let msg = msg_of(r);
    let h: <MinSig as Variant>::Signature =
        ops::hash_with_namespace::<MinSig>(dst_of(r), &ns, &msg);
    let comp: [u8; 48] = h.encode().as_ref().try_into().expect("G1 hash is 48 B");
    signature_compressed_to_eip2537(&comp).expect("hash is a valid G1 point")
}

const EXPECTED_H: &[&str] = &[
    "000000000000000000000000000000001123a1bf57c9b9c44a80d453d11f3a006e69f8a76447a77ae5b12151e2f77f175212264b1f247f14854b4228134e26030000000000000000000000000000000005dff48ee582b254859f3ae7c6517cca20a765d443f0ff3570a020710340036b8132985936811892140df3254d826651",
    "0000000000000000000000000000000008946433018a17d3063648f3619d43df7decab5af6c2386f6f1c521619dcda4225be0b22ef926b6d1ceaead5cc6ffc340000000000000000000000000000000002af7b8ada472e08de992fa07603f51a1a3af99dbd5c0a5238c3f28a7e0990ba4e57b9bb174af6b5826db77f55472007",
    "000000000000000000000000000000000721ba0b4e1ad4d630bcc7e1058b6d35572c7993014060b677957b9975a5d65de1cd082e499c245659af92a2d7f195000000000000000000000000000000000002fc749cccd738bd027e4d60d6cfe5e903ded681795313979c0260e467da04c129b90a62750a6e4804d0856780e96f20",
    "00000000000000000000000000000000159ae2356a8ea6b357ec562d1e1fa31ca0165c1c9bac4ef67603f7f3a40361a92d2ec5b39447fcb9699f7ce53b4b859e0000000000000000000000000000000001f630e8b43cebf3b42e647571b7136b2d4279903c3c6c5f88d23bba5d3a71ea2822dfea45c024bb5f1044311004b44e",
    "000000000000000000000000000000000124d66d4398ab8e1c815ff26a6fd5ba8d40e033c47ac97e6063db6c0145987c27efe6d505776543c74d754b4fd20f6800000000000000000000000000000000047535cc58be29cb65abc68f10c67b26d860c22c28aab281e7dffc98b54692c871a8eba60256cabbe2e18e73d0f998de",
    "000000000000000000000000000000000d55c8664d8df37c52c4d54e7d9d3c768dfdade91cf7ebe817b0cd5a672fa80a30bc2098a59cf1bf87238d1392f9337500000000000000000000000000000000049cd5bb24ceea9cf25370adefd2bbb91aea9121d213543c4b11541b818b86ef817b020e966d07fc5134f0e50e1a9033",
];

// Verify tuples use the PoP DST: verify = hashToG1 + pairing, and the only
// DST-dependent half (hashToG1) is already pinned for both DSTs above.

const OFF: u64 = 1_000_000;

struct VerifyRecipe {
    label: &'static str,
    seed: u64,
    chain_id: u64,
    tampered: bool,
}

const VERIFY_RECIPES: &[VerifyRecipe] = &[
    VerifyRecipe {
        label: "verify_pop_valid",
        seed: 0,
        chain_id: C_MAIN,
        tampered: false,
    },
    VerifyRecipe {
        label: "verify_pop_tampered_sig",
        seed: 0,
        chain_id: C_MAIN,
        tampered: true,
    },
];

fn kp(seed: u64) -> ValidatorBlsKeypair {
    ValidatorBlsKeypair::generate(&mut StdRng::seed_from_u64(seed))
}

/// (ns, msg=pk_comp, sigUnc128, sigRef48, pkUnc256, pkRef96, expectedValid)
type VerifyTuple = (
    Vec<u8>,
    [u8; 96],
    [u8; 128],
    [u8; 48],
    [u8; 256],
    [u8; 96],
    bool,
);

fn verify_tuple(v: &VerifyRecipe) -> VerifyTuple {
    let k = kp(v.seed);
    let ns = fluent_namespace(v.chain_id);
    let pk_ref: [u8; 96] = k.public_bytes();
    let pk_unc = pubkey_compressed_to_eip2537(&pk_ref).expect("valid G2");
    // Tampered rows carry a valid signature from another key, so encoding and
    // pairing run and only the pairing check fails.
    let sig_ref: [u8; 48] = if v.tampered {
        sign_pop(&kp(v.seed + OFF), &ns)
    } else {
        sign_pop(&k, &ns)
    };
    let sig_unc = signature_compressed_to_eip2537(&sig_ref).expect("valid G1");
    (ns, pk_ref, sig_unc, sig_ref, pk_unc, pk_ref, !v.tampered)
}

// Per VERIFY_RECIPES row: (sigUnc128, sigRef48, pkUnc256, pkRef96) hex.
const VERIFY_EXPECTED: &[(&str, &str, &str, &str)] = &[
    ("00000000000000000000000000000000027ecd57f1889127d81b2a3c46e1905c419302192ebc90f818c7d272b38a6495337f7dde0733d0d431fc1338e8caf62e00000000000000000000000000000000109a4722abb94b2ffb8685abe75b4fc8336d2f6534b64fee49baa07ab7357de65036fb93ee119860768cc65daa4c7b1e", "a27ecd57f1889127d81b2a3c46e1905c419302192ebc90f818c7d272b38a6495337f7dde0733d0d431fc1338e8caf62e", "000000000000000000000000000000000727ef1c60e48042142f7bcc8b6382305cd50c5a4542c44ec72a4de6640c194f8ef36bea1dbed168ab6fd8681d910d550000000000000000000000000000000012b050b6fbe80695b5d56835e978918e37c8707a7fad09a01ae782d4c3170c9baa4c2c196b36eac6b78ceb210b287aeb000000000000000000000000000000000f9da5ef5089f62dc55ec91c2459f6ed3fd9981f8d4926ad90dca0314603ae4af86c8fa12bdd2569867f05a24908b7fc0000000000000000000000000000000009ac1ba2c6341d99ba0d6bfab8ea6a3a58726e787ab22b899cd95acfec350c1fc09f5fcbbef992106b61e45eb9158354", "92b050b6fbe80695b5d56835e978918e37c8707a7fad09a01ae782d4c3170c9baa4c2c196b36eac6b78ceb210b287aeb0727ef1c60e48042142f7bcc8b6382305cd50c5a4542c44ec72a4de6640c194f8ef36bea1dbed168ab6fd8681d910d55"),
    ("000000000000000000000000000000001733f7c8769099b3c5f2601d80aec5f35b4e0086b9d4f2092140e0f40002c328ceb71b469d9456ed4caa27e340a78d9b00000000000000000000000000000000058d6642e4126b5d37407dcb4a34911ceab3992b1524ce67bf5fdd2688374a692839dba9ac3ba6ab3305cf51200d49ca", "9733f7c8769099b3c5f2601d80aec5f35b4e0086b9d4f2092140e0f40002c328ceb71b469d9456ed4caa27e340a78d9b", "000000000000000000000000000000000727ef1c60e48042142f7bcc8b6382305cd50c5a4542c44ec72a4de6640c194f8ef36bea1dbed168ab6fd8681d910d550000000000000000000000000000000012b050b6fbe80695b5d56835e978918e37c8707a7fad09a01ae782d4c3170c9baa4c2c196b36eac6b78ceb210b287aeb000000000000000000000000000000000f9da5ef5089f62dc55ec91c2459f6ed3fd9981f8d4926ad90dca0314603ae4af86c8fa12bdd2569867f05a24908b7fc0000000000000000000000000000000009ac1ba2c6341d99ba0d6bfab8ea6a3a58726e787ab22b899cd95acfec350c1fc09f5fcbbef992106b61e45eb9158354", "92b050b6fbe80695b5d56835e978918e37c8707a7fad09a01ae782d4c3170c9baa4c2c196b36eac6b78ceb210b287aeb0727ef1c60e48042142f7bcc8b6382305cd50c5a4542c44ec72a4de6640c194f8ef36bea1dbed168ab6fd8681d910d55"),
];

fn dehex<const N: usize>(s: &str) -> [u8; N] {
    let v = hex::decode(s).expect("valid hex");
    v.try_into().expect("length mismatch")
}

#[test]
fn conformance_corpus_matches_committed_constants() {
    assert_eq!(
        EXPECTED_H.len(),
        RECIPES.len(),
        "EXPECTED_H and RECIPES out of sync"
    );
    for (r, e) in RECIPES.iter().zip(EXPECTED_H.iter()) {
        assert_eq!(
            hash_eip2537(r),
            dehex::<128>(e),
            "hash-to-G1 corpus drift at `{}` — Commonware/blst pipeline \
             changed; regenerate via `--ignored print_corpus`, review the \
             diff, and mirror into the Solidity test in the same PR",
            r.label
        );
    }
}

#[test]
fn verify_corpus_matches_committed_constants() {
    assert_eq!(
        VERIFY_EXPECTED.len(),
        VERIFY_RECIPES.len(),
        "VERIFY_EXPECTED and VERIFY_RECIPES out of sync"
    );
    for (v, e) in VERIFY_RECIPES.iter().zip(VERIFY_EXPECTED.iter()) {
        let (ns, _msg, sig_unc, sig_ref, pk_unc, pk_ref, expected_valid) = verify_tuple(v);
        assert_eq!(
            sig_unc,
            dehex::<128>(e.0),
            "verify drift (sigUnc) at `{}`",
            v.label
        );
        assert_eq!(
            sig_ref,
            dehex::<48>(e.1),
            "verify drift (sigRef) at `{}`",
            v.label
        );
        assert_eq!(
            pk_unc,
            dehex::<256>(e.2),
            "verify drift (pkUnc) at `{}`",
            v.label
        );
        assert_eq!(
            pk_ref,
            dehex::<96>(e.3),
            "verify drift (pkRef) at `{}`",
            v.label
        );

        let verify_ok = fluentbase_bls::pop::verify_pop(&pk_ref, &ns, &sig_ref).is_ok();
        assert_eq!(
            verify_ok, expected_valid,
            "verify_pop disagrees with recipe `{}`: got={}, expected={}",
            v.label, verify_ok, expected_valid,
        );
    }
}

/// Regenerator: prints both corpora to paste into `EXPECTED_H` / `VERIFY_EXPECTED` and the mirror.
#[test]
#[ignore = "regenerator: prints the pinned corpus for hand-mirroring"]
fn print_corpus() {
    println!("\nconst EXPECTED_H: &[&str] = &[");
    for r in RECIPES {
        println!("    \"{}\", // {}", hex::encode(hash_eip2537(r)), r.label);
    }
    println!("];");
    // Preimages for the Solidity mirror, which cannot run rand_08.
    println!("\n// hashToG1 Solidity mirror inputs (label | ns_hex | msg_hex | dst):");
    for r in RECIPES {
        let dst = if matches!(r.subject, Subject::Pop) {
            "POP"
        } else {
            "SIG"
        };
        println!(
            "// {} | {} | {} | {}",
            r.label,
            hex::encode(ns_of(r)),
            hex::encode(msg_of(r)),
            dst
        );
    }

    println!("\nconst VERIFY_EXPECTED: &[(&str, &str, &str, &str)] = &[");
    for v in VERIFY_RECIPES {
        let (_ns, _msg, sig_unc, sig_ref, pk_unc, pk_ref, _valid) = verify_tuple(v);
        println!(
            "    (\"{}\", \"{}\", \"{}\", \"{}\"), // {}",
            hex::encode(sig_unc),
            hex::encode(sig_ref),
            hex::encode(pk_unc),
            hex::encode(pk_ref),
            v.label
        );
    }
    println!("];");
    println!(
        "\n// verify Solidity mirror (label | ns_hex | msg_hex(=pkRef) | dst=POP | expectedValid):"
    );
    for v in VERIFY_RECIPES {
        let (ns, msg, _su, _sr, _pu, _pr, valid) = verify_tuple(v);
        println!(
            "// {} | {} | {} | POP | {}",
            v.label,
            hex::encode(ns),
            hex::encode(msg),
            valid
        );
    }
}
