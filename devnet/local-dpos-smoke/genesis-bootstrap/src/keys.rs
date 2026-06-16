// Phase 2 scaffolding: `KeySet.chain_id` is read by Phase 4
// (`pop::produce` uses it to namespace BLS PoP signatures).
#![allow(dead_code)]

use alloy_signer_local::PrivateKeySigner;
use coins_bip39::{English, Mnemonic};
use commonware_codec::DecodeExt;
use commonware_cryptography::bls12381::{
    dkg::deal_anonymous,
    primitives::{group::Share, sharing::Sharing, variant::MinSig},
};
use commonware_cryptography::ed25519::PrivateKey as Ed25519PrivateKey;
use commonware_utils::N3f1;
use eyre::{OptionExt, WrapErr};
use fluentbase_bls::keys::ValidatorBlsKeypair;
use rand_08::rngs::StdRng;
use rand_core::SeedableRng;
use sha2::{Digest, Sha256};
use std::num::NonZeroU32;

pub struct Validator {
    pub idx: u32,
    pub l2_signer: PrivateKeySigner,
    pub bls: ValidatorBlsKeypair,
    pub peer: Ed25519PrivateKey,
    pub slasher: PrivateKeySigner,
    pub slasher_password: String,
    pub reth_p2p: secp256k1::SecretKey,
    /// This validator's threshold-beacon DKG share (devnet bootstrap: the key
    /// is dealt deterministically at genesis, mirroring the epoch-0 committee
    /// bootstrap, since the live DKG actor is phased — research Q2). Signs the
    /// per-height seed partials (`prev_randao(h) = H(seed(h))`).
    pub beacon_share: Share,
}

pub struct FullNode {
    pub peer: Ed25519PrivateKey,
}

pub struct KeySet {
    pub chain_id: u64,
    pub validators: Vec<Validator>,
    pub full_node: FullNode,
    pub governance_signer: PrivateKeySigner,
    /// The dealt beacon sharing — its `.public()` is `PK_epoch`, published to L2
    /// (`commitEpochBeaconKey`) and the group key every node verifies seeds
    /// against. The same `Sharing` underlies all validators' [`Validator::beacon_share`].
    pub beacon_sharing: Sharing<MinSig>,
}

pub fn derive(mnemonic: &str, peers: u32, chain_id: u64) -> eyre::Result<KeySet> {
    let m = Mnemonic::<English>::new_from_phrase(mnemonic).wrap_err("invalid BIP39 mnemonic")?;
    let seed = m
        .to_seed(None)
        .map_err(|e| eyre::eyre!("BIP39 to_seed failed: {e:?}"))?;

    let derive_32 = |role: &str, idx: u32| -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(b"fluent-dpos-smoke-v1");
        h.update(seed.as_slice());
        h.update(b"|");
        h.update(role.as_bytes());
        h.update(b"|");
        h.update(idx.to_be_bytes());
        h.finalize().into()
    };

    // Deal the threshold randomness-beacon key across all `peers` validators,
    // deterministically from the mnemonic so every regen is reproducible. The
    // share index assigned here is the signer index that appears in seed
    // partials; recovery only needs the shared `Sharing`, so it is independent
    // of the (peer-pubkey-sorted) committee ordering.
    let beacon_n = NonZeroU32::new(peers).ok_or_eyre("peers must be non-zero for beacon DKG")?;
    let mut beacon_rng = StdRng::from_seed(derive_32("beacon-dkg", 0));
    let (beacon_sharing, beacon_shares) =
        deal_anonymous::<MinSig, N3f1>(&mut beacon_rng, Default::default(), beacon_n);

    let mut validators = Vec::with_capacity(peers as usize);
    for i in 0..peers {
        let l2_bytes = derive_32("l2-owner", i);
        let bls_seed = derive_32("bls", i);
        let peer_bytes = derive_32("peer", i);
        let slasher_bytes = derive_32("slasher", i);

        let l2_signer =
            PrivateKeySigner::from_bytes(&l2_bytes.into()).wrap_err("derive L2 signer")?;
        // BLS12-381 scalar field is smaller than 2^256, so a raw 32-byte
        // seed can fall outside [1, r). `ValidatorBlsKeypair::generate`
        // wraps the IETF BLS KeyGen retry-on-zero loop; seeding StdRng
        // deterministically preserves reproducibility.
        let mut bls_rng = StdRng::from_seed(bls_seed);
        let bls = ValidatorBlsKeypair::generate(&mut bls_rng);
        let peer = Ed25519PrivateKey::decode(peer_bytes.as_slice())
            .map_err(|e| eyre::eyre!("derive ed25519 peer key: {e:?}"))?;
        let slasher = PrivateKeySigner::from_bytes(&slasher_bytes.into())
            .wrap_err("derive slasher signer")?;
        let slasher_password = hex::encode(derive_32("slasher-pwd", i));
        // reth p2p secret key — secp256k1 over the same deterministic seed.
        // Used by validator-0 for its enode identity so followers can pin a
        // `--trusted-peers=enode://<pubkey>@<ip>:30303` URL and complete
        // historical sync after they connect (sequencer-url's
        // `eth_subscribe("newHeads")` only carries live blocks; without P2P
        // catch-up, followers stay at block 0 — see [[reth-sequencer-url-no-backfill]]).
        let reth_p2p_bytes = derive_32("reth-p2p", i);
        let reth_p2p = secp256k1::SecretKey::from_byte_array(reth_p2p_bytes)
            .wrap_err("derive reth p2p secp256k1 key")?;

        validators.push(Validator {
            idx: i,
            l2_signer,
            bls,
            peer,
            slasher,
            slasher_password,
            reth_p2p,
            beacon_share: beacon_shares[i as usize].clone(),
        });
    }

    let full_node_bytes = derive_32("peer-full", 0);
    let full_node = FullNode {
        peer: Ed25519PrivateKey::decode(full_node_bytes.as_slice())
            .map_err(|e| eyre::eyre!("derive ed25519 full-node key: {e:?}"))?,
    };

    let governance_bytes = derive_32("governance", 0);
    let governance_signer = PrivateKeySigner::from_bytes(&governance_bytes.into())
        .wrap_err("derive governance signer")?;

    Ok(KeySet {
        chain_id,
        validators,
        full_node,
        governance_signer,
        beacon_sharing,
    })
}
