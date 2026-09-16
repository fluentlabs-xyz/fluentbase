//! Slashing-evidence bridge: convert Simplex `Activity::Conflicting*` events into
//! the four `slashEquivocation*` calldata arguments expected by `Staking.sol`.
//!
//! This is a stateless module: every helper takes the evidence struct plus a
//! per-epoch committee `BiMap` and returns the byte arrays the on-chain entry
//! points expect.
//!
//! # Pipeline overview
//!
//! ```text
//!   commonware Activity::Conflicting* (from simplex batcher)
//!     │  evidence = ev.encode()                      (Commonware codec)
//!     │  signer_idx = ev.signer() (via Attributable trait)
//!     │  re-decode inner Notarize/Finalize/Nullify for sig access
//!     │    sig_g1 = .attestation.signature.get()?     (force lazy decode)
//!     │    sig_compressed = sig_g1.encode() as [u8; 48]
//!     │  pk_compressed = committee.value(signer_idx)?.encode() as [u8; 96]
//!     │  EIP-2537 conversions (signature_compressed_to_eip2537 +
//!     │                        pubkey_compressed_to_eip2537)
//!     ▼
//!   SlashCallArgs { kind, evidence (raw), pk_uncompressed (256B),
//!                   sig1_uncompressed (128B), sig2_uncompressed (128B) }
//!     │
//!     ▼
//!   ABI-encode + submit `slashEquivocation*(evidence, pkUnc, sig1Unc, sig2Unc)`
//!   (caller responsibility — slasher worker)
//! ```
//!
//! ## Why re-decode instead of accessor methods
//!
//! `ConflictingNotarize`, `ConflictingFinalize` and `NullifyFinalize` keep their
//! inner fields private and expose no accessors, but their `Write`/`Read` impls
//! are public, so the inner signature material is reached by an encode/decode
//! round-trip. Equivocation events are rare, so the extra pass is irrelevant.

use commonware_codec::{Encode, Read as _};
use commonware_consensus::{
    simplex::types::{
        Activity, Attributable, ConflictingFinalize, ConflictingNotarize, Finalize, Notarize,
        Nullify, NullifyFinalize,
    },
    Epochable,
};
use commonware_cryptography::{certificate, certificate::Attestation, Digest as DigestTrait};
use commonware_parallel::Sequential;
use commonware_utils::ordered::BiMap;
use fluentbase_bls::{
    combined_scheme::CombinedSignature,
    encoding::{pubkey_compressed_to_eip2537, signature_compressed_to_eip2537},
    fluent_namespace, BlsPubkey, EpochCommittee, Error, PeerPubkey, Scheme, VoteScheme,
    PUBKEY_BYTES, PUBKEY_EIP2537_BYTES, SIGNATURE_BYTES, SIGNATURE_EIP2537_BYTES,
};
use fluentbase_p2p::constants::MAX_COMMITTEE_SIZE;
use rand_core::{CryptoRngCore, OsRng};

/// Discriminator for the three `slashEquivocation*` entry points, pairing 1:1 with
/// the `Staking.sol` functions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlashKind {
    ConflictingNotarize,
    ConflictingFinalize,
    NullifyFinalize,
}

/// Decode cap for the equivocation charge an [`crate::order_block::OrderBlock`]
/// carries: a one-byte tag plus two single-signer votes, each a round, a 32-byte
/// proposal digest, a signer index and a 97-byte `CombinedSignature`, so ~290 B at
/// the widest. 1 KiB leaves generous headroom and keeps the block's equivocation
/// carve-out negligible.
pub const MAX_EQUIVOCATION_SIZE: usize = 1024;

/// Why a block-carried equivocation charge was refused.
///
/// Every variant is a vote-false, and they are named apart only so the log line
/// says which one: the first three mean the proposer sent bytes no committee
/// could act on, the last three mean it named a fault that did not happen.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ChargeError {
    #[error("evidence is not a decodable Activity")]
    Undecodable,
    #[error("trailing bytes after the evidence")]
    TrailingBytes,
    #[error("evidence is not one of the three attributable Byzantine variants")]
    NotAttributable,
    #[error("evidence is for epoch {charged}, the block is in epoch {block}")]
    EpochMismatch { charged: u64, block: u64 },
    #[error("evidence attributes signer {signer}, the block accuses {accused}")]
    SignerMismatch { signer: u32, accused: u8 },
    #[error("evidence carries a signature that does not verify")]
    BadSignature,
}

/// Verify the charge a block carries against the committee of the block's own
/// epoch — the vote-time gate every committee member runs before backing the
/// block (`application::equivocation_gate_decision`).
///
/// A charge for any other epoch is refused: only the committee of a charge's own
/// epoch can verify it (a past epoch's `signer_idx → BLS key` mapping is not
/// reconstructible from a running node), which is why a charge that outlives its
/// epoch leaves by the transaction fallback instead.
///
/// The decode does the structural work: `Conflicting*::read_cfg` re-checks the
/// same signer/round/proposal invariants `Conflicting*::new` asserts, so
/// well-formedness and attribution are settled before any pairing is trusted.
pub fn verify_block_charge(
    evidence: &[u8],
    accused: u8,
    epoch: u64,
    committee: BiMap<PeerPubkey, BlsPubkey>,
    chain_id: u64,
) -> Result<(), ChargeError> {
    let mut buf = evidence;
    // The cfg bounds a certificate variant's signer bitmap; those variants are
    // refused below, but the bound must hold at decode time so a peer cannot buy an
    // unbounded allocation with a one-byte tag.
    let activity = Activity::<Scheme, crate::digest::Digest>::read_cfg(
        &mut buf,
        &(MAX_COMMITTEE_SIZE as usize),
    )
    .map_err(|_| ChargeError::Undecodable)?;
    if !buf.is_empty() {
        return Err(ChargeError::TrailingBytes);
    }
    if SlashKind::from_activity(&activity).is_none() {
        return Err(ChargeError::NotAttributable);
    }
    let charged = activity.epoch().get();
    if charged != epoch {
        return Err(ChargeError::EpochMismatch {
            charged,
            block: epoch,
        });
    }
    let signer = attributable_signer_idx(&activity).ok_or(ChargeError::NotAttributable)?;
    if signer != u32::from(accused) {
        return Err(ChargeError::SignerMismatch { signer, accused });
    }
    let vote_scheme = VoteScheme::verifier(&fluent_namespace(chain_id), committee);
    verify_pre_submit_vote_only(&activity, &vote_scheme, &mut OsRng)
        .map_err(|_| ChargeError::BadSignature)
}

/// The committee position a slashable `Activity` attributes its fault to.
///
/// `None` for every other variant, including the certificates: those carry an
/// aggregated signer bitmap rather than one attributable index, so there is
/// nobody to charge.
pub fn attributable_signer_idx<S, D>(activity: &Activity<S, D>) -> Option<u32>
where
    S: certificate::Scheme,
    D: DigestTrait,
{
    match activity {
        Activity::ConflictingNotarize(ev) => Some(ev.signer().get()),
        Activity::ConflictingFinalize(ev) => Some(ev.signer().get()),
        Activity::NullifyFinalize(ev) => Some(ev.signer().get()),
        _ => None,
    }
}

impl SlashKind {
    /// Filter an `Activity` event to its `SlashKind`, or `None` if not a
    /// slashable variant.
    ///
    /// ```ignore
    /// if let Some(kind) = SlashKind::from_activity(&activity) {
    ///     // dispatch to extract_from_* + ABI-encode + submit
    /// }
    /// ```
    pub fn from_activity<S, D>(activity: &Activity<S, D>) -> Option<Self>
    where
        S: certificate::Scheme,
        D: DigestTrait,
    {
        match activity {
            Activity::ConflictingNotarize(_) => Some(Self::ConflictingNotarize),
            Activity::ConflictingFinalize(_) => Some(Self::ConflictingFinalize),
            Activity::NullifyFinalize(_) => Some(Self::NullifyFinalize),
            _ => None,
        }
    }
}

/// The four byte-shaped arguments expected by the on-chain
/// `slashEquivocation*` entry points.
///
/// - `evidence`: raw Commonware-encoded `ConflictingNotarize` /
///   `ConflictingFinalize` / `NullifyFinalize` bytes (variant-length;
///   ~135–168 B for the standard 32-B digest case).
/// - `pk_uncompressed`: offender's BLS12-381 G2 public key in EIP-2537
///   uncompressed format (256 B).
/// - `sig1_uncompressed` / `sig2_uncompressed`: the two BLS12-381 G1
///   signatures in EIP-2537 uncompressed format (128 B each).
#[derive(Clone, Debug)]
pub struct SlashCallArgs {
    pub kind: SlashKind,
    pub evidence: Vec<u8>,
    pub pk_uncompressed: [u8; PUBKEY_EIP2537_BYTES],
    pub sig1_uncompressed: [u8; SIGNATURE_EIP2537_BYTES],
    pub sig2_uncompressed: [u8; SIGNATURE_EIP2537_BYTES],
}

/// Encode the attributable multisig half of a combined-scheme signature (G1
/// compressed, 48 B) into the fixed buffer the contract expects. The per-vote
/// signature is `CombinedSignature{vote, seed}`; equivocation evidence is over the
/// `vote` half only — the threshold `seed` partial is non-attributable and never
/// submitted on-chain.
fn sig_compressed(sig: &CombinedSignature) -> Result<[u8; SIGNATURE_BYTES], Error> {
    sig.vote()
        .encode()
        .as_ref()
        .try_into()
        .map_err(|_| Error::InvalidSignature)
}

/// Project a combined-scheme attestation onto its attributable multisig
/// (`VoteScheme`) half, dropping the non-attributable threshold seed partial.
/// Re-encoding the evidence over `VoteScheme` keeps each signature 48 B on the
/// wire — the layout `SimplexEvidenceDecoder.sol` parses (`uvarint(signer) ‖
/// sig[48]`). A raw `ConflictingNotarize<Scheme>::encode()` would desync the
/// on-chain decoder.
fn vote_attestation(att: &Attestation<Scheme>) -> Result<Attestation<VoteScheme>, Error> {
    let combined = att.signature.get().ok_or(Error::InvalidSignature)?;
    Ok(Attestation {
        signer: att.signer,
        signature: (*combined.vote()).into(),
    })
}

/// Look up the offender's `BlsPubkey` (96 B G2 compressed) by
/// `signer_idx`. The index must be within `committee.len()`.
fn pk_compressed(
    signer_idx: u32,
    committee: &BiMap<PeerPubkey, BlsPubkey>,
) -> Result<[u8; PUBKEY_BYTES], Error> {
    let bls_pk: &BlsPubkey =
        committee
            .value(signer_idx as usize)
            .ok_or(Error::SignerIndexOutOfRange {
                signer_idx,
                committee_len: committee.len(),
            })?;
    bls_pk
        .encode()
        .as_ref()
        .try_into()
        .map_err(|_| Error::InvalidPubkey)
}

/// Assert the activity's claimed epoch matches the committee's epoch
/// before extracting — protects against passing a mis-aligned committee
/// to the per-variant extractors.
#[inline]
fn check_epoch_match(ev: &impl Epochable, committee: &EpochCommittee) -> Result<(), Error> {
    let evidence_epoch = ev.epoch().get();
    if evidence_epoch != committee.epoch {
        return Err(Error::EpochMismatch {
            evidence_epoch,
            committee_epoch: committee.epoch,
        });
    }
    Ok(())
}

/// Build `slashEquivocationNotarize` calldata args from a
/// `ConflictingNotarize` event.
pub fn extract_from_conflicting_notarize<D>(
    ev: &ConflictingNotarize<Scheme, D>,
    committee: &EpochCommittee,
) -> Result<SlashCallArgs, Error>
where
    D: DigestTrait,
{
    check_epoch_match(ev, committee)?;
    let raw = ev.encode().to_vec();

    // Re-decode the two inner Notarize structs; `Conflicting*` fields are private,
    // so the public `Write`/`Read` traits are the only path to the inner signatures.
    let mut buf: &[u8] = &raw;
    let n1 = Notarize::<Scheme, D>::read_cfg(&mut buf, &()).map_err(|_| Error::InvalidSignature)?;
    let n2 = Notarize::<Scheme, D>::read_cfg(&mut buf, &()).map_err(|_| Error::InvalidSignature)?;

    // Re-establish the equivocation structural invariant before paying gas; it
    // mirrors commonware's `ConflictingNotarize::new`/`read_cfg`, and `verify` does
    // not re-check it. Compare round (epoch+view), not just view.
    if n1.signer() != n2.signer() || n1.round() != n2.round() || n1.proposal == n2.proposal {
        return Err(Error::NonConflictingEvidence);
    }

    let sig1_g1 = n1
        .attestation
        .signature
        .get()
        .ok_or(Error::InvalidSignature)?;
    let sig2_g1 = n2
        .attestation
        .signature
        .get()
        .ok_or(Error::InvalidSignature)?;

    let sig1 = sig_compressed(sig1_g1)?;
    let sig2 = sig_compressed(sig2_g1)?;
    let pk96 = pk_compressed(ev.signer().get(), &committee.bimap)?;

    // Re-encode over `VoteScheme` so each signature is the 48-byte multisig half
    // the on-chain decoder expects.
    let evidence = ConflictingNotarize::<VoteScheme, D>::new(
        Notarize {
            proposal: n1.proposal.clone(),
            attestation: vote_attestation(&n1.attestation)?,
        },
        Notarize {
            proposal: n2.proposal.clone(),
            attestation: vote_attestation(&n2.attestation)?,
        },
    )
    .encode()
    .to_vec();

    Ok(SlashCallArgs {
        kind: SlashKind::ConflictingNotarize,
        evidence,
        pk_uncompressed: pubkey_compressed_to_eip2537(&pk96)?,
        sig1_uncompressed: signature_compressed_to_eip2537(&sig1)?,
        sig2_uncompressed: signature_compressed_to_eip2537(&sig2)?,
    })
}

/// Build `slashEquivocationFinalize` calldata args from a
/// `ConflictingFinalize` event.
pub fn extract_from_conflicting_finalize<D>(
    ev: &ConflictingFinalize<Scheme, D>,
    committee: &EpochCommittee,
) -> Result<SlashCallArgs, Error>
where
    D: DigestTrait,
{
    check_epoch_match(ev, committee)?;
    let raw = ev.encode().to_vec();

    let mut buf: &[u8] = &raw;
    let f1 = Finalize::<Scheme, D>::read_cfg(&mut buf, &()).map_err(|_| Error::InvalidSignature)?;
    let f2 = Finalize::<Scheme, D>::read_cfg(&mut buf, &()).map_err(|_| Error::InvalidSignature)?;

    // Structural invariant (mirrors `ConflictingFinalize::new`/`read_cfg`).
    if f1.signer() != f2.signer() || f1.round() != f2.round() || f1.proposal == f2.proposal {
        return Err(Error::NonConflictingEvidence);
    }

    let sig1_g1 = f1
        .attestation
        .signature
        .get()
        .ok_or(Error::InvalidSignature)?;
    let sig2_g1 = f2
        .attestation
        .signature
        .get()
        .ok_or(Error::InvalidSignature)?;

    let sig1 = sig_compressed(sig1_g1)?;
    let sig2 = sig_compressed(sig2_g1)?;
    let pk96 = pk_compressed(ev.signer().get(), &committee.bimap)?;

    // Re-encode over `VoteScheme` (48-byte-per-signature wire layout) — see
    // `extract_from_conflicting_notarize`.
    let evidence = ConflictingFinalize::<VoteScheme, D>::new(
        Finalize {
            proposal: f1.proposal.clone(),
            attestation: vote_attestation(&f1.attestation)?,
        },
        Finalize {
            proposal: f2.proposal.clone(),
            attestation: vote_attestation(&f2.attestation)?,
        },
    )
    .encode()
    .to_vec();

    Ok(SlashCallArgs {
        kind: SlashKind::ConflictingFinalize,
        evidence,
        pk_uncompressed: pubkey_compressed_to_eip2537(&pk96)?,
        sig1_uncompressed: signature_compressed_to_eip2537(&sig1)?,
        sig2_uncompressed: signature_compressed_to_eip2537(&sig2)?,
    })
}

/// Build `slashEquivocationNullifyFinalize` calldata args from a
/// `NullifyFinalize` event.
pub fn extract_from_nullify_finalize<D>(
    ev: &NullifyFinalize<Scheme, D>,
    committee: &EpochCommittee,
) -> Result<SlashCallArgs, Error>
where
    D: DigestTrait,
{
    check_epoch_match(ev, committee)?;
    let raw = ev.encode().to_vec();

    let mut buf: &[u8] = &raw;
    let nullify =
        Nullify::<Scheme>::read_cfg(&mut buf, &()).map_err(|_| Error::InvalidSignature)?;
    let finalize =
        Finalize::<Scheme, D>::read_cfg(&mut buf, &()).map_err(|_| Error::InvalidSignature)?;

    // Structural invariant (mirrors `NullifyFinalize::new`/`read_cfg`): same signer
    // and same round. There is no proposals-differ check — a nullify has no
    // proposal, and adding one would over-reject valid evidence.
    if nullify.signer() != finalize.signer() || nullify.round != finalize.round() {
        return Err(Error::NonConflictingEvidence);
    }

    let sig1_g1 = nullify
        .attestation
        .signature
        .get()
        .ok_or(Error::InvalidSignature)?;
    let sig2_g1 = finalize
        .attestation
        .signature
        .get()
        .ok_or(Error::InvalidSignature)?;

    let sig1 = sig_compressed(sig1_g1)?;
    let sig2 = sig_compressed(sig2_g1)?;
    let pk96 = pk_compressed(ev.signer().get(), &committee.bimap)?;

    // Re-encode over `VoteScheme` (48-byte-per-signature wire layout) — see
    // `extract_from_conflicting_notarize`.
    let evidence = NullifyFinalize::<VoteScheme, D>::new(
        Nullify {
            round: nullify.round,
            attestation: vote_attestation(&nullify.attestation)?,
        },
        Finalize {
            proposal: finalize.proposal.clone(),
            attestation: vote_attestation(&finalize.attestation)?,
        },
    )
    .encode()
    .to_vec();

    Ok(SlashCallArgs {
        kind: SlashKind::NullifyFinalize,
        evidence,
        pk_uncompressed: pubkey_compressed_to_eip2537(&pk96)?,
        sig1_uncompressed: signature_compressed_to_eip2537(&sig1)?,
        sig2_uncompressed: signature_compressed_to_eip2537(&sig2)?,
    })
}

/// Local cryptographic verify of an `Activity` event before submitting it
/// on-chain. Wraps `Activity::verify` with the `Sequential` strategy (one Activity
/// is two pairings, so fork-join overhead exceeds the benefit).
///
/// Does not verify equivocation invariants (already enforced by
/// `Conflicting*::new` and `Read::read_cfg`) or on-chain state (validator
/// registration, tombstone) — those are Solidity concerns.
pub fn verify_pre_submit<D, R>(
    activity: &Activity<Scheme, D>,
    scheme: &Scheme,
    rng: &mut R,
) -> Result<(), Error>
where
    D: DigestTrait,
    R: CryptoRngCore,
{
    if activity.verify(rng, scheme, &Sequential) {
        Ok(())
    } else {
        Err(Error::InvalidSignature)
    }
}

/// Verify an attributable `Activity<Scheme, D>` — a slashable `Conflicting*` pair or
/// a single `Notarize`/`Finalize`/`Nullify` off the evidence channel — against a
/// `VoteScheme` verifier built from the committee bimap.
///
/// This is the verify path used when the per-epoch `Scheme` is pruned from the
/// scheme provider and must be rebuilt from the recovered committee bimap: the
/// per-epoch DKG polynomial needed by [`Scheme::verify_attestation`]'s seeded arm
/// is not recoverable at slash time, so a rebuilt `build_verifier(.., None)` would
/// wrongly reject a seeded vote on a beacon-active chain. The on-chain evidence is
/// over the vote half only, so verifying just that half is the correct and
/// sufficient pre-submit check.
///
/// The projection drops the seed partial and reconstructs the
/// `Activity<VoteScheme, D>` so `Activity::verify` checks the same
/// signer/round/proposal-bound signatures the on-chain decoder will see. An
/// equivocation attestation is single-signer, not a bitmap.
pub fn verify_pre_submit_vote_only<D, R>(
    activity: &Activity<Scheme, D>,
    vote_scheme: &VoteScheme,
    rng: &mut R,
) -> Result<(), Error>
where
    D: DigestTrait,
    R: CryptoRngCore,
{
    let vote_activity = project_activity_to_vote(activity)?;
    if vote_activity.verify(rng, vote_scheme, &Sequential) {
        Ok(())
    } else {
        Err(Error::InvalidSignature)
    }
}

/// Re-project an `Activity<Scheme, D>` onto `Activity<VoteScheme, D>`,
/// dropping the threshold seed half of every attestation. Mirrors the per-variant
/// `extract_from_*` re-encoding, but returns the typed Activity so it can be
/// re-verified rather than ABI-encoded.
///
/// The three single-vote variants are here for the evidence channel
/// ([`crate::slasher::gossip`]), which verifies one peer-forwarded vote at a time;
/// their inner fields are public, so they need no encode round-trip.
fn project_activity_to_vote<D>(
    activity: &Activity<Scheme, D>,
) -> Result<Activity<VoteScheme, D>, Error>
where
    D: DigestTrait,
{
    match activity {
        Activity::ConflictingNotarize(ev) => {
            let raw = ev.encode().to_vec();
            let mut buf: &[u8] = &raw;
            let n1 = Notarize::<Scheme, D>::read_cfg(&mut buf, &())
                .map_err(|_| Error::InvalidSignature)?;
            let n2 = Notarize::<Scheme, D>::read_cfg(&mut buf, &())
                .map_err(|_| Error::InvalidSignature)?;
            Ok(Activity::ConflictingNotarize(ConflictingNotarize::<
                VoteScheme,
                D,
            >::new(
                Notarize {
                    proposal: n1.proposal.clone(),
                    attestation: vote_attestation(&n1.attestation)?,
                },
                Notarize {
                    proposal: n2.proposal.clone(),
                    attestation: vote_attestation(&n2.attestation)?,
                },
            )))
        }
        Activity::ConflictingFinalize(ev) => {
            let raw = ev.encode().to_vec();
            let mut buf: &[u8] = &raw;
            let f1 = Finalize::<Scheme, D>::read_cfg(&mut buf, &())
                .map_err(|_| Error::InvalidSignature)?;
            let f2 = Finalize::<Scheme, D>::read_cfg(&mut buf, &())
                .map_err(|_| Error::InvalidSignature)?;
            Ok(Activity::ConflictingFinalize(ConflictingFinalize::<
                VoteScheme,
                D,
            >::new(
                Finalize {
                    proposal: f1.proposal.clone(),
                    attestation: vote_attestation(&f1.attestation)?,
                },
                Finalize {
                    proposal: f2.proposal.clone(),
                    attestation: vote_attestation(&f2.attestation)?,
                },
            )))
        }
        Activity::NullifyFinalize(ev) => {
            let raw = ev.encode().to_vec();
            let mut buf: &[u8] = &raw;
            let nullify =
                Nullify::<Scheme>::read_cfg(&mut buf, &()).map_err(|_| Error::InvalidSignature)?;
            let finalize = Finalize::<Scheme, D>::read_cfg(&mut buf, &())
                .map_err(|_| Error::InvalidSignature)?;
            Ok(Activity::NullifyFinalize(
                NullifyFinalize::<VoteScheme, D>::new(
                    Nullify {
                        round: nullify.round,
                        attestation: vote_attestation(&nullify.attestation)?,
                    },
                    Finalize {
                        proposal: finalize.proposal.clone(),
                        attestation: vote_attestation(&finalize.attestation)?,
                    },
                ),
            ))
        }
        Activity::Notarize(n) => Ok(Activity::Notarize(Notarize {
            proposal: n.proposal.clone(),
            attestation: vote_attestation(&n.attestation)?,
        })),
        Activity::Finalize(f) => Ok(Activity::Finalize(Finalize {
            proposal: f.proposal.clone(),
            attestation: vote_attestation(&f.attestation)?,
        })),
        Activity::Nullify(n) => Ok(Activity::Nullify(Nullify {
            round: n.round,
            attestation: vote_attestation(&n.attestation)?,
        })),
        // A certificate variant carries an aggregated bitmap, not an attributable
        // single-signer attestation, so it has no vote-only projection.
        _ => Err(Error::NonConflictingEvidence),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_codec::DecodeExt as _;
    use commonware_consensus::{
        simplex::types::Proposal,
        types::{Epoch, Round, View},
    };
    use commonware_cryptography::certificate::Scheme as SchemeTrait;
    use commonware_cryptography::{
        bls12381::{dkg::deal_anonymous, primitives::variant::MinSig},
        ed25519::PrivateKey as Ed25519PrivateKey,
        sha256::Digest as Sha256Digest,
        Signer,
    };
    use commonware_math::algebra::Random;
    use commonware_utils::{N3f1, TryCollect};
    use fluentbase_bls::{
        beacon::seed_namespace, fluent_namespace, keys::ValidatorBlsKeypair, scheme::build_signer,
    };
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng;

    const TEST_CHAIN_ID: u64 = 20_994;

    fn small_committee(
        seed: u64,
        n: usize,
    ) -> (Vec<ValidatorBlsKeypair>, BiMap<PeerPubkey, BlsPubkey>) {
        let mut rng = StdRng::seed_from_u64(seed);
        let peer_sks: Vec<_> = (0..n)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let bls_kps: Vec<_> = (0..n)
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

    fn digest(tag: u8) -> Sha256Digest {
        let mut d = [0u8; 32];
        d[0] = tag;
        d[31] = tag;
        Sha256Digest::from(d)
    }

    fn round() -> Round {
        Round::new(Epoch::new(EV_EPOCH), View::new(42))
    }

    /// The epoch every fixture here signs at — and, since a scheme is bound to
    /// one epoch, the epoch every fixture scheme is built for.
    const EV_EPOCH: u64 = 7;

    fn build_ev_conflicting_notarize(
        seed: u64,
    ) -> (
        ConflictingNotarize<Scheme, Sha256Digest>,
        BiMap<PeerPubkey, BlsPubkey>,
    ) {
        let (kps, bimap) = small_committee(seed, 4);
        let s = build_signer(
            &fluent_namespace(TEST_CHAIN_ID),
            bimap.clone(),
            &kps[0],
            EV_EPOCH,
            None,
        )
        .expect("offender must be member");
        let p1 = Proposal::new(round(), View::new(41), digest(0xaa));
        let p2 = Proposal::new(round(), View::new(41), digest(0xbb));
        let n1 = Notarize::sign(&s, p1).expect("sign n1");
        let n2 = Notarize::sign(&s, p2).expect("sign n2");
        (ConflictingNotarize::new(n1, n2), bimap)
    }

    #[test]
    fn from_activity_returns_none_for_non_conflicting_variants() {
        let (ev, _) = build_ev_conflicting_notarize(1);
        let conflicting = Activity::<Scheme, Sha256Digest>::ConflictingNotarize(ev);
        assert_eq!(
            SlashKind::from_activity(&conflicting),
            Some(SlashKind::ConflictingNotarize)
        );

        let (kps, bimap) = small_committee(2, 4);
        let s = build_signer(
            &fluent_namespace(TEST_CHAIN_ID),
            bimap,
            &kps[0],
            EV_EPOCH,
            None,
        )
        .unwrap();
        let n = Notarize::sign(&s, Proposal::new(round(), View::new(41), digest(0xcc))).unwrap();
        let plain = Activity::<Scheme, Sha256Digest>::Notarize(n);
        assert_eq!(SlashKind::from_activity(&plain), None);
    }

    #[test]
    fn extract_returns_signer_index_out_of_range_for_empty_committee() {
        let (ev, _) = build_ev_conflicting_notarize(1);
        // Same epoch as the event so the `SignerIndexOutOfRange` path is reached.
        let empty_committee = EpochCommittee::from_unverified(7, BiMap::default());
        let err = extract_from_conflicting_notarize(&ev, &empty_committee)
            .expect_err("must reject signer_idx >= empty bimap");
        assert!(
            matches!(
                err,
                Error::SignerIndexOutOfRange {
                    committee_len: 0,
                    ..
                }
            ),
            "got: {err:?}"
        );
    }

    #[test]
    fn extract_rejects_epoch_mismatch() {
        // The event is at epoch 7; an epoch-8 committee must short-circuit with
        // `EpochMismatch` before the signer-index lookup.
        let (ev, bimap) = build_ev_conflicting_notarize(1);
        let wrong_epoch_committee = EpochCommittee::from_unverified(8, bimap);
        let err = extract_from_conflicting_notarize(&ev, &wrong_epoch_committee)
            .expect_err("must reject committee from wrong epoch");
        assert!(
            matches!(
                err,
                Error::EpochMismatch {
                    evidence_epoch: 7,
                    committee_epoch: 8
                }
            ),
            "got: {err:?}"
        );
    }

    #[test]
    fn verify_pre_submit_rejects_tampered_signature() {
        let (ev, bimap) = build_ev_conflicting_notarize(1);
        let activity = Activity::<Scheme, Sha256Digest>::ConflictingNotarize(ev);

        let scheme = fluentbase_bls::scheme::build_verifier(
            &fluent_namespace(TEST_CHAIN_ID),
            bimap,
            EV_EPOCH,
            None,
        );
        let mut rng = StdRng::seed_from_u64(0xdeadbeef);

        verify_pre_submit(&activity, &scheme, &mut rng).expect("clean activity must verify");

        // Flip a byte in sig1's body (offset 50): the Read invariants (signer,
        // round) stay intact but the pairing check fails.
        let bytes = match &activity {
            Activity::ConflictingNotarize(ev) => ev.encode().to_vec(),
            _ => unreachable!(),
        };
        let mut tampered = bytes.clone();
        tampered[50] ^= 0x01;
        let tampered_ev: ConflictingNotarize<Scheme, Sha256Digest> =
            ConflictingNotarize::decode(tampered.as_slice())
                .expect("tampering a sig body byte must keep Read invariants intact");
        let tampered_activity = Activity::<Scheme, Sha256Digest>::ConflictingNotarize(tampered_ev);

        let err = verify_pre_submit(&tampered_activity, &scheme, &mut rng)
            .expect_err("tampered activity must fail verify");
        assert!(matches!(err, Error::InvalidSignature), "got: {err:?}");
    }

    /// Build a seeded (beacon-active) `ConflictingNotarize`: the offender signs two
    /// conflicting proposals at the same round with a combined scheme that carries a
    /// real threshold seed partial, so `Notarize` votes carry
    /// `CombinedSignature{vote, seed: Some(..)}`.
    fn build_seeded_ev_conflicting_notarize(
        seed: u64,
    ) -> (
        ConflictingNotarize<Scheme, Sha256Digest>,
        BiMap<PeerPubkey, BlsPubkey>,
    ) {
        let (kps, bimap) = small_committee(seed, 4);
        let mut rng = StdRng::seed_from_u64(seed ^ 0x5eed);
        let (sharing, shares) =
            deal_anonymous::<MinSig, N3f1>(&mut rng, Default::default(), 4u32.try_into().unwrap());
        let ns = fluent_namespace(TEST_CHAIN_ID);
        let seed_ns = seed_namespace(&ns);
        // The threshold share index must equal the offender's Simplex participant
        // index (its BiMap slot), so resolve it through the signer's `me()`.
        let probe = build_signer(&ns, bimap.clone(), &kps[0], EV_EPOCH, None)
            .expect("offender is a member");
        let me = SchemeTrait::me(&probe).expect("signer carries a Participant index");
        let share = shares.iter().find(|sh| sh.index == me).unwrap().clone();
        let oracle: std::sync::Arc<dyn fluentbase_bls::oracle::SeedOracle> =
            std::sync::Arc::new(crate::beacon::testing::DealtOracle {
                sharing,
                share: Some(share),
                namespace: seed_ns,
            });
        let seeded = build_signer(&ns, bimap.clone(), &kps[0], EV_EPOCH, Some(oracle))
            .expect("offender must be member");
        let p1 = Proposal::new(round(), View::new(41), digest(0xaa));
        let p2 = Proposal::new(round(), View::new(41), digest(0xbb));
        let n1 = Notarize::sign(&seeded, p1).expect("sign n1");
        let n2 = Notarize::sign(&seeded, p2).expect("sign n2");
        (ConflictingNotarize::new(n1, n2), bimap)
    }

    #[test]
    fn vote_only_verify_accepts_seeded_evidence_that_combined_none_verifier_rejects() {
        // On a beacon-active chain the offender's votes carry a seed partial. A
        // rebuilt combined verifier with `beacon = None` (all that is recoverable at
        // slash time) rejects them; the vote-only path must accept them by checking
        // just the attributable multisig half.
        let (ev, bimap) = build_seeded_ev_conflicting_notarize(7);
        let activity = Activity::<Scheme, Sha256Digest>::ConflictingNotarize(ev);
        let mut rng = StdRng::seed_from_u64(0xabcd);

        let combined_none = fluentbase_bls::scheme::build_verifier(
            &fluent_namespace(TEST_CHAIN_ID),
            bimap.clone(),
            EV_EPOCH,
            None,
        );
        assert!(
            verify_pre_submit(&activity, &combined_none, &mut rng).is_err(),
            "combined verifier with beacon=None must reject seeded evidence (the regression)"
        );

        let vote_scheme = VoteScheme::verifier(&fluent_namespace(TEST_CHAIN_ID), bimap);
        verify_pre_submit_vote_only(&activity, &vote_scheme, &mut rng)
            .expect("vote-only verify must accept seeded equivocation evidence");
    }

    #[test]
    fn vote_only_verify_rejects_tampered_seeded_evidence() {
        // The vote-only path must still reject a corrupted vote-half signature: it
        // is a real crypto check, not a structural rubber-stamp.
        let (ev, bimap) = build_seeded_ev_conflicting_notarize(11);
        let activity = Activity::<Scheme, Sha256Digest>::ConflictingNotarize(ev);
        let mut rng = StdRng::seed_from_u64(0x1234);
        let vote_scheme = VoteScheme::verifier(&fluent_namespace(TEST_CHAIN_ID), bimap.clone());

        verify_pre_submit_vote_only(&activity, &vote_scheme, &mut rng)
            .expect("clean seeded evidence must verify vote-only");

        // Tamper a byte in sig1's vote body, then re-decode and re-verify.
        let bytes = match &activity {
            Activity::ConflictingNotarize(ev) => ev.encode().to_vec(),
            _ => unreachable!(),
        };
        let mut tampered = bytes.clone();
        tampered[50] ^= 0x01;
        let tampered_ev: ConflictingNotarize<Scheme, Sha256Digest> =
            ConflictingNotarize::decode(tampered.as_slice())
                .expect("tampering a sig body byte must keep Read invariants intact");
        let tampered_activity = Activity::<Scheme, Sha256Digest>::ConflictingNotarize(tampered_ev);
        let err = verify_pre_submit_vote_only(&tampered_activity, &vote_scheme, &mut rng)
            .expect_err("tampered vote-half must fail vote-only verify");
        assert!(matches!(err, Error::InvalidSignature), "got: {err:?}");
    }

    #[test]
    fn extract_returns_signer_index_out_of_range_for_short_committee() {
        // `build_ev_conflicting_notarize(1)` sorts the offender to `signer_idx = 3`.
        let (ev, _) = build_ev_conflicting_notarize(1);

        // A 2-member bimap puts signer_idx=3 out of range regardless of peer keys.
        // The epoch matches the event so the lookup path is reached, not the epoch
        // short-circuit.
        let (_, short_bimap) = small_committee(99, 2);
        let short_committee = EpochCommittee::from_unverified(7, short_bimap);

        let err = extract_from_conflicting_notarize(&ev, &short_committee)
            .expect_err("must reject signer_idx=3 >= short_bimap.len()=2");
        assert!(
            matches!(
                err,
                Error::SignerIndexOutOfRange {
                    signer_idx: 3,
                    committee_len: 2,
                }
            ),
            "got: {err:?}"
        );
    }

    /// A block-charge fixture over the block digest type the `OrderBlock` carries;
    /// the tests above use a Sha256 digest, which `verify_block_charge` does not
    /// accept.
    fn block_charge(
        seed: u64,
    ) -> (
        Activity<Scheme, crate::digest::Digest>,
        BiMap<PeerPubkey, BlsPubkey>,
        u32,
    ) {
        let (kps, bimap) = small_committee(seed, 4);
        let signer = build_signer(
            &fluent_namespace(TEST_CHAIN_ID),
            bimap.clone(),
            &kps[0],
            EV_EPOCH,
            None,
        )
        .expect("offender must be a committee member");
        let n1 = Notarize::sign(
            &signer,
            Proposal::new(
                round(),
                View::new(41),
                crate::digest::Digest(alloy_primitives::B256::repeat_byte(0xaa)),
            ),
        )
        .expect("offender signs");
        let n2 = Notarize::sign(
            &signer,
            Proposal::new(
                round(),
                View::new(41),
                crate::digest::Digest(alloy_primitives::B256::repeat_byte(0xbb)),
            ),
        )
        .expect("offender signs");
        let signer_idx = n1.signer().get();
        (
            Activity::ConflictingNotarize(ConflictingNotarize::new(n1, n2)),
            bimap,
            signer_idx,
        )
    }

    /// The decode cap is a wire budget carved out of every block's tx budget, so
    /// it must stay far above what a real charge actually costs — and must never
    /// drift below it.
    #[test]
    fn an_assembled_charge_encodes_well_under_the_block_cap() {
        let (charge, _, _) = block_charge(11);
        let len = charge.encode().len();
        assert!(
            (200..=400).contains(&len),
            "a ConflictingNotarize is ~290 B; got {len} — re-check MAX_EQUIVOCATION_SIZE"
        );
        assert!(
            len * 2 < MAX_EQUIVOCATION_SIZE,
            "the cap must keep real headroom"
        );
    }

    #[test]
    fn verify_block_charge_accepts_a_real_charge() {
        let (charge, bimap, signer_idx) = block_charge(12);
        assert_eq!(
            verify_block_charge(
                &charge.encode(),
                signer_idx as u8,
                round().epoch().get(),
                bimap,
                TEST_CHAIN_ID,
            ),
            Ok(())
        );
    }

    #[test]
    fn verify_block_charge_rejects_every_way_a_charge_can_be_wrong() {
        let (charge, bimap, signer_idx) = block_charge(13);
        let bytes = charge.encode();
        let accused = signer_idx as u8;
        let epoch = round().epoch().get();
        let check = |bytes: &[u8], accused: u8, epoch: u64, chain_id: u64| {
            verify_block_charge(bytes, accused, epoch, bimap.clone(), chain_id).unwrap_err()
        };

        assert_eq!(
            check(&[], accused, epoch, TEST_CHAIN_ID),
            ChargeError::Undecodable
        );

        let mut trailing = bytes.to_vec();
        trailing.push(0);
        assert_eq!(
            check(&trailing, accused, epoch, TEST_CHAIN_ID),
            ChargeError::TrailingBytes
        );

        // A lone vote is well-formed and correctly signed, but it is not evidence
        // of anything.
        let (kps, plain_bimap) = small_committee(13, 4);
        let signer = build_signer(
            &fluent_namespace(TEST_CHAIN_ID),
            plain_bimap,
            &kps[0],
            EV_EPOCH,
            None,
        )
        .unwrap();
        let lone = Activity::<Scheme, crate::digest::Digest>::Notarize(
            Notarize::sign(
                &signer,
                Proposal::new(
                    round(),
                    View::new(41),
                    crate::digest::Digest(alloy_primitives::B256::repeat_byte(0xaa)),
                ),
            )
            .unwrap(),
        );
        assert_eq!(
            check(&lone.encode(), accused, epoch, TEST_CHAIN_ID),
            ChargeError::NotAttributable
        );

        // Only the committee of a charge's own epoch can verify it, so a charge from
        // any other epoch is refused outright.
        assert_eq!(
            check(&bytes, accused, epoch + 1, TEST_CHAIN_ID),
            ChargeError::EpochMismatch {
                charged: epoch,
                block: epoch + 1,
            }
        );

        // Real evidence, but the block names somebody else.
        let innocent = accused.wrapping_add(1);
        assert_eq!(
            check(&bytes, innocent, epoch, TEST_CHAIN_ID),
            ChargeError::SignerMismatch {
                signer: signer_idx,
                accused: innocent,
            }
        );

        // Signatures bound to another chain's domain separator.
        assert_eq!(
            check(&bytes, accused, epoch, TEST_CHAIN_ID + 1),
            ChargeError::BadSignature
        );
    }
}
