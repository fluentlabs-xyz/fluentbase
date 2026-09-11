//! Consensus-key registration and deterministic epoch committee commits.

use crate::{
    bls,
    consts::*,
    events,
    evidence::{self, EvidenceShape},
    math,
    staking::{
        remove_active, remove_delegation_from_totals, selection_visible_at, set_selection_visible,
        top_k_by_stake_at, validator_status,
    },
    storage::{chain_config_storage, consensus_storage, staking_storage},
    types::{AddressCommand, ConsensusKeys, EpochSignerCommand, EquivocationCommand, U64Command},
    util::{
        current_epoch, decode, decode_args, ensure_initialized, ensure_mutable, ensure_non_payable,
        next_epoch, revert, revert_with, try_transfer, write_abi, write_returns,
    },
};
use alloc::vec::Vec;
use fluentbase_sdk::{
    keccak256, Address, Bytes, ContextReader, ExitCode, SharedAPI, Uint, B256, U256,
};

const BLS_POP_DST: &[u8] = b"BLS_POP_BLS12381G1_XMD:SHA-256_SSWU_RO_POP_";

fn read_consensus_keys<SDK: SharedAPI>(
    sdk: &SDK,
    validator: Address,
) -> Result<ConsensusKeys, ExitCode> {
    let keys = consensus_storage()
        .consensus_keys_accessor()
        .entry(validator);
    let peer_pubkey = keys.peer_pubkey_accessor().get_checked(sdk)?;
    if peer_pubkey.is_zero() {
        return Ok(ConsensusKeys::default());
    }
    Ok(ConsensusKeys {
        bls_pubkey: read_bls_pubkey(sdk, validator)?,
        peer_pubkey,
        activation_epoch: keys.activation_epoch_accessor().get_checked(sdk)?,
    })
}

pub(crate) fn read_bls_pubkey<SDK: SharedAPI>(
    sdk: &SDK,
    validator: Address,
) -> Result<Bytes, ExitCode> {
    let parts = consensus_storage()
        .consensus_keys_accessor()
        .entry(validator)
        .bls_pubkey_accessor();
    let mut key = Vec::with_capacity(BLS_PUBKEY_LENGTH);
    for index in 0..BLS_PUBKEY_WORDS {
        key.extend_from_slice(parts.at(index).get_checked(sdk)?.as_slice());
    }
    Ok(Bytes::from(key))
}

fn fluent_namespace<SDK: SharedAPI>(sdk: &SDK) -> Bytes {
    let mut namespace = b"FLUENT_DPOS_V1_".to_vec();
    namespace.extend_from_slice(&sdk.context().block_chain_id().to_be_bytes());
    Bytes::from(namespace)
}

pub(crate) struct VerifiedConsensusKeys {
    bls_pubkey: [B256; BLS_PUBKEY_WORDS],
    bls_pubkey_hash: B256,
    encoded_bls_pubkey: Bytes,
    peer_pubkey: B256,
}

/// Validates a validator's proof of possession and converts its BLS key to the
/// fixed three-word representation used in storage.
pub(crate) fn verify_consensus_keys<SDK: SharedAPI>(
    sdk: &mut SDK,
    validator: Address,
    bls_pubkey_uncompressed: Bytes,
    bls_pop_uncompressed: Bytes,
    peer_pubkey: B256,
) -> Result<VerifiedConsensusKeys, ExitCode> {
    let consensus = consensus_storage();
    if consensus
        .tombstoned_accessor()
        .entry(validator)
        .get_checked(sdk)?
    {
        return revert_with(sdk, ERR_ALREADY_SLASHED_FOR_EQUIVOCATION, &validator);
    }
    if bls_pubkey_uncompressed.len() != BLS_PUBKEY_UNCOMPRESSED_LENGTH
        || bls_pop_uncompressed.len() != BLS_POP_UNCOMPRESSED_LENGTH
        || peer_pubkey.is_zero()
    {
        return revert(sdk, ERR_INVALID_CONSENSUS_KEY_ENCODING);
    }
    if !consensus
        .peer_pubkey_owner_accessor()
        .entry(peer_pubkey)
        .get_checked(sdk)?
        .is_zero()
    {
        return revert_with(sdk, ERR_PEER_PUBKEY_ALREADY_IN_USE, &peer_pubkey);
    }

    let compressed = bls::compress_g2_unchecked(sdk, &bls_pubkey_uncompressed)?;
    let bls_pubkey_hash = keccak256(compressed);
    if !consensus
        .bls_pubkey_owner_accessor()
        .entry(bls_pubkey_hash)
        .get_checked(sdk)?
        .is_zero()
    {
        return revert_with(sdk, ERR_BLS_PUBKEY_ALREADY_IN_USE, &bls_pubkey_hash);
    }
    // The compressed key is what the node registered its identity under, and it
    // is what goes into the signed message — the PoP is a signature over the
    // validator's own public key. The uncompressed bytes go in beside it, and
    // PAIRING is what ties the two together: an attacker who finds a second
    // 256-byte preimage compressing to a registered key still has to pass a
    // pairing over THOSE bytes, which rejects a non-canonical coordinate and a
    // dirty EIP-2537 pad.
    let valid = bls::verify(
        sdk,
        &fluent_namespace(sdk),
        &compressed,
        BLS_POP_DST,
        &bls_pop_uncompressed,
        &bls_pubkey_uncompressed,
    )?;
    if !valid {
        return revert_with(sdk, ERR_INVALID_PROOF_OF_POSSESSION, &validator);
    }

    // `compress_g2_unchecked` returns exactly `BLS_PUBKEY_WORDS` whole words, so
    // the chunking cannot leave a remainder.
    let mut bls_pubkey = [B256::ZERO; BLS_PUBKEY_WORDS];
    for (word, chunk) in bls_pubkey
        .iter_mut()
        .zip(compressed.chunks_exact(U256::BYTES))
    {
        *word = B256::from_slice(chunk);
    }

    Ok(VerifiedConsensusKeys {
        bls_pubkey,
        bls_pubkey_hash,
        encoded_bls_pubkey: Bytes::copy_from_slice(&compressed),
        peer_pubkey,
    })
}

/// Stores already-verified keys atomically with validator creation.
///
/// **Caller contract: run `verify_consensus_keys` on these exact bytes
/// immediately before, with nothing between that could claim a key.** Both
/// callers do (`initializer.rs`, `staking.rs`), and the `set_validator` that sits
/// between them in each touches neither owner map — `staking.rs` never names
/// them.
///
/// Written down because the two rechecks that used to enforce it here were
/// removed on 2026-09-11 as unreachable duplicates, and the only local witness
/// went with them. The writes below are unconditional, and `peer_pubkey_owner` /
/// `bls_pubkey_owner` are WRITE-ONCE by assumption alone. `slash_from_evidence`
/// resolves an equivocator's identity through `bls_pubkey_owner` precisely
/// because nothing releases it, so a third caller that skipped the verification
/// would not merely corrupt a registration — it would send a later slash to the
/// wrong validator.
pub(crate) fn store_consensus_keys<SDK: SharedAPI>(
    sdk: &mut SDK,
    validator: Address,
    verified: VerifiedConsensusKeys,
    activation_epoch: u64,
) -> Result<(), ExitCode> {
    let consensus = consensus_storage();
    let keys = consensus.consensus_keys_accessor().entry(validator);
    if !keys.peer_pubkey_accessor().get_checked(sdk)?.is_zero() {
        return revert_with(sdk, ERR_CONSENSUS_KEYS_ALREADY_SET, &validator);
    }
    // The peer-key and BLS-key uniqueness rechecks that used to stand here are
    // gone. They duplicated `verify_consensus_keys`, which every caller runs
    // immediately before this, and they were unreachable: they were written when
    // the verifier was an external contract that could reenter between the two,
    // and that reason left with the external call. Both errors are still raised
    // — by `verify_consensus_keys`, which is where the uniqueness is decided.
    let parts = keys.bls_pubkey_accessor();
    for (index, part) in verified.bls_pubkey.into_iter().enumerate() {
        parts.at(index).set_checked(sdk, part)?;
    }
    keys.peer_pubkey_accessor()
        .set_checked(sdk, verified.peer_pubkey)?;
    keys.activation_epoch_accessor()
        .set_checked(sdk, activation_epoch)?;
    consensus
        .peer_pubkey_owner_accessor()
        .entry(verified.peer_pubkey)
        .set_checked(sdk, validator)?;
    consensus
        .bls_pubkey_owner_accessor()
        .entry(verified.bls_pubkey_hash)
        .set_checked(sdk, validator)?;
    events::ConsensusKeysSet {
        validator,
        bls_pubkey: verified.encoded_bls_pubkey,
        peer_pubkey: verified.peer_pubkey,
        activation_epoch,
    }
    .emit(sdk)
}

/// Public handler `0xad36f42f` (`getConsensusKeys`).
///
/// Returns a validator's consensus keys and activation epoch.
pub fn get_consensus_keys<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    let keys = read_consensus_keys(sdk, decode::<AddressCommand>(input)?.value)?;
    write_abi(sdk, &keys)
}

fn write_validators_with_keys<SDK: SharedAPI>(
    sdk: &mut SDK,
    validators: Vec<Address>,
) -> Result<(), ExitCode> {
    let mut keys = Vec::with_capacity(validators.len());
    for validator in &validators {
        keys.push(read_consensus_keys(sdk, *validator)?);
    }
    write_returns(sdk, &(validators, keys))
}

/// Public handler `0xd96cbd7b` (`getRegistryWithKeys`).
///
/// Returns all registered validators together with their consensus keys.
pub fn get_registry_with_keys<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    let active = staking_storage().active_validators_accessor();
    let len = active.len_checked(sdk)?;
    let mut validators = Vec::with_capacity(len as usize);
    for index in 0..len {
        validators.push(active.at(index).get_checked(sdk)?);
    }
    write_validators_with_keys(sdk, validators)
}

/// Public handler `0xc06a82de` (`nextEpochToCommit`).
///
/// Returns the next epoch whose committee can be committed.
pub fn next_epoch_to_commit<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    write_abi(
        sdk,
        &consensus_storage()
            .last_committed_epoch_p1_accessor()
            .get_checked(sdk)?,
    )
}

/// Which membership record `epoch` seats, and how many of its entries.
///
/// The length comes from the index, not the record — see the field docs for why
/// that is a hot-path choice rather than a divergence guard. `length == 0` is
/// the sole encoding of "not committed"; `record == 0` is a real pointer,
/// because genesis mints record 0.
///
/// Two SLOADs: the halves share a slot but `get_checked` reads per field. Use
/// [`committee_length_at`] where the pointer is not wanted.
/// How many members `epoch` seats, without touching the record pointer.
///
/// `record_production` runs on every block and wants the length alone. The two
/// halves share a slot but not a read: `get_checked` is one `read_at(slot,
/// offset)` per field, so [`committee_at`] costs two SLOADs where this costs one.
pub(crate) fn committee_length_at<SDK: SharedAPI>(sdk: &SDK, epoch: u64) -> Result<u64, ExitCode> {
    Ok(consensus_storage()
        .epoch_index_accessor()
        .entry(epoch)
        .length_accessor()
        .get_checked(sdk)? as u64)
}

pub(crate) fn committee_at<SDK: SharedAPI>(sdk: &SDK, epoch: u64) -> Result<(u64, u64), ExitCode> {
    let index = consensus_storage().epoch_index_accessor().entry(epoch);
    let length = index.length_accessor().get_checked(sdk)? as u64;
    let record = index.record_accessor().get_checked(sdk)? as u64;
    Ok((record, length))
}

pub(crate) fn committee_changed<SDK: SharedAPI>(
    sdk: &SDK,
    target: u64,
    members: &[CommitteeMember],
) -> Result<bool, ExitCode> {
    if target == 0 {
        return Ok(false);
    }
    // The index's length, NOT the incumbent record's own: the record is shared
    // across epochs and a shorter successor must still read as changed.
    let (record, length) = committee_at(sdk, target - 1)?;
    if length as usize != members.len() {
        return Ok(true);
    }
    // Positional, and both sides are peer-key ordered, so an unchanged set
    // cannot read as changed. This holds for all time only because the sort key
    // is immutable — see the consensus-key writer, which never releases a peer
    // key. Allowing key rotation would void it with no change here.
    let incumbent = consensus_storage()
        .committee_records_accessor()
        .entry(record);
    for (index, member) in members.iter().enumerate() {
        if incumbent.at(index as u64).get_checked(sdk)? != member.validator {
            return Ok(true);
        }
    }
    Ok(false)
}

/// First ring slot of `epoch`'s frame.
const fn ring_base(epoch: u64) -> usize {
    (epoch % WEIGHT_RING_EPOCHS) as usize * PAIRS_MAX
}

/// Freeze `members`' leader weights into `epoch`'s ring frame.
///
/// Writes are contiguous from index 0 of the frame, which is what lets a reader
/// trust pair 0's stamp — see [`read_weights`].
fn write_ring<SDK: SharedAPI>(
    sdk: &mut SDK,
    epoch: u64,
    members: &[CommitteeMember],
) -> Result<(), ExitCode> {
    // See `ERR_COMMITTEE_EXCEEDS_WEIGHT_RING`: an assertion of the cap held two
    // layers up, kept local because overrunning the frame corrupts the NEXT
    // epoch's weights silently. Checked before the compaction pass, so an
    // over-long committee is named as one rather than as an overflow.
    if members.len() > PAIRS_MAX * 2 {
        return revert_with(
            sdk,
            ERR_COMMITTEE_EXCEEDS_WEIGHT_RING,
            &(U256::from(members.len()), U256::from(PAIRS_MAX * 2)),
        );
    }
    let mut weights = Vec::with_capacity(members.len());
    for member in members {
        weights.push(math::compact_balance(member.weight).ok_or(ExitCode::IntegerOverflow)?);
    }
    let ring = consensus_storage().weight_ring_accessor();
    let base = ring_base(epoch);
    let stamp = epoch as u32;
    let mut index = 0;
    while index < weights.len() {
        let pair = ring.at(base + index / 2);
        pair.a_accessor().set_checked(sdk, weights[index])?;
        // On an odd count the second half is written to zero rather than left
        // holding the previous occupant's weight under this epoch's fresh stamp.
        // Nothing reads it — the count bound stops one short — but writing it
        // makes that state stop existing, so the bound becomes a belt rather than
        // the only guard. One warm store into a slot this frame has already
        // dirtied.
        let b = weights.get(index + 1).copied().unwrap_or(Uint::ZERO);
        pair.b_accessor().set_checked(sdk, b)?;
        pair.stamp_accessor().set_checked(sdk, stamp)?;
        index += 2;
    }
    Ok(())
}

/// Frozen leader weights for `epoch`, or `None` once the ring has wrapped past
/// it.
///
/// Bounded by `epoch_index[epoch].length` — **never** by the ring's extent, and
/// never by scanning stamps. The count is the authority; the stamp only
/// invalidates.
///
/// **The stamp checked is pair 0's, and "the last pair read" is the unsafe
/// reading this excludes by name.** Ring writes for one epoch are contiguous
/// from index 0 within its frame, and `MIN_COMMITTEE_LENGTH` guarantees at least
/// two pairs, so every occupant of a frame writes pair 0 — pair 0's stamp is the
/// frame's most recent occupant. Concretely: let E seat 51 (26 pairs) and
/// `E + N` seat 4 (2 pairs). `E + N` overwrites pairs 0–1 and leaves pairs 2–25
/// carrying E's stamp and E's weights. A reader for E that checked its last pair
/// would find stamp E, conclude the frame is E's, and then read pairs 0–1 — which
/// now hold `E + N`'s weights. A confidently wrong answer, which is the exact
/// failure the stamp exists to prevent.
///
/// An epoch with no committee has no weights and that is not a ring miss, so it
/// answers `Some(vec![])`.
pub(crate) fn read_weights<SDK: SharedAPI>(
    sdk: &SDK,
    epoch: u64,
) -> Result<Option<Vec<math::U112>>, ExitCode> {
    let (_, length) = committee_at(sdk, epoch)?;
    if length == 0 {
        return Ok(Some(Vec::new()));
    }
    let ring = consensus_storage().weight_ring_accessor();
    let base = ring_base(epoch);
    if ring.at(base).stamp_accessor().get_checked(sdk)? != epoch as u32 {
        return Ok(None);
    }
    let mut weights = Vec::with_capacity(length as usize);
    for index in 0..length as usize {
        let pair = ring.at(base + index / 2);
        weights.push(if index % 2 == 0 {
            pair.a_accessor().get_checked(sdk)?
        } else {
            pair.b_accessor().get_checked(sdk)?
        });
    }
    Ok(Some(weights))
}

/// The peer key of `validator`, if its consensus keys are active at `epoch`.
///
/// Deliberately narrower than `read_consensus_keys`, which also loads the
/// three-word BLS public key: committee selection never looks at that key, and
/// this runs once per selected candidate on the commit path. The getters that
/// do return the BLS key keep using the wider reader.
fn active_peer_key_at<SDK: SharedAPI>(
    sdk: &SDK,
    validator: Address,
    epoch: u64,
) -> Result<Option<B256>, ExitCode> {
    let keys = consensus_storage()
        .consensus_keys_accessor()
        .entry(validator);
    let peer_pubkey = keys.peer_pubkey_accessor().get_checked(sdk)?;
    if peer_pubkey.is_zero() {
        return Ok(None);
    }
    let activation_epoch = keys.activation_epoch_accessor().get_checked(sdk)?;
    Ok((activation_epoch <= epoch).then_some(peer_pubkey))
}

/// A selected committee member with the two values the commit would otherwise
/// have to read a second time.
pub(crate) struct CommitteeMember {
    pub validator: Address,
    pub peer_pubkey: B256,
    pub weight: U256,
}

/// Committee for `epoch`: the Active validators that are ELIGIBLE at `epoch`,
/// ranked by stake and cut to the configured cap.
///
/// Eligible means all three of: status Active, selection-visible at `epoch` (the
/// production-exclusion stamp), and holding a consensus key active by `epoch`.
///
/// The whole filter runs *before* the stake cut. A validator that cannot be
/// seated occupying a slot in the cut and being dropped afterwards spent that
/// seat on nobody — the seat was not passed to the next eligible validator, and
/// a population of twenty eligible validators could seat four. Filtering first
/// is what makes the cut hand out `min(cap, eligible)` seats to validators that
/// can actually take them, and what lets a production exclusion free a seat for
/// the next candidate instead of shrinking the committee.
pub(crate) fn selected_committee_at<SDK: SharedAPI>(
    sdk: &SDK,
    epoch: u64,
) -> Result<Vec<CommitteeMember>, ExitCode> {
    let active = staking_storage().active_validators_accessor();
    let active_len = active.len_checked(sdk)?;
    let mut candidates = Vec::with_capacity(active_len as usize);
    for index in 0..active_len {
        let validator = active.at(index).get_checked(sdk)?;
        // The `STATUS_ACTIVE` term is an ASSERTION of an invariant, not a live
        // filter, and it is kept as one rather than removed. Only ACTIVE
        // validators enter `active_validators` (`staking.rs`) and every
        // transition out of ACTIVE calls `remove_active`, so no mutation of this
        // term can be caught by a test that goes through the API — measured, not
        // assumed: removing it leaves the whole suite green (R1.5a, and again
        // this session). What it still catches is a write straight to storage
        // that leaves a non-ACTIVE validator in the list; `eligible_population_at_least`
        // carries the same term for exactly that state and HAS a test for it, so
        // dropping it here alone would make the two disagree about who is
        // eligible.
        if validator_status(sdk, validator)? != STATUS_ACTIVE
            || !selection_visible_at(sdk, validator, epoch)?
        {
            continue;
        }
        if let Some(peer_pubkey) = active_peer_key_at(sdk, validator, epoch)? {
            candidates.push((validator, peer_pubkey));
        }
    }
    let cap = chain_config_storage()
        .active_validators_length_accessor()
        .get_checked(sdk)? as usize;
    let ranked = top_k_by_stake_at(
        sdk,
        candidates.iter().map(|(validator, _)| *validator).collect(),
        epoch,
        cap,
    )?;
    let mut members = Vec::with_capacity(ranked.len());
    for (validator, weight) in ranked {
        // `ranked` is a subset of `candidates`, so the key is always found; the
        // key was read in the filter pass above rather than a second time here,
        // which is one fewer SLOAD pair per seated member.
        let peer_pubkey = candidates
            .iter()
            .find(|(candidate, _)| *candidate == validator)
            .map(|(_, peer_pubkey)| *peer_pubkey)
            .ok_or(ExitCode::Panic)?;
        members.push(CommitteeMember {
            validator,
            peer_pubkey,
            weight,
        });
    }
    Ok(members)
}

/// Public handler `0xe505b249` (`commitEpochCommittee`).
///
/// Derives the next epoch's committee and freezes it, with its leader weights.
///
/// Takes no argument. It used to accept the committee and check it against the
/// set derived here — but that set is the authority the check compared against,
/// so the check could only ever detect a disagreement with the *caller's*
/// derivation, and its only remedy was to revert. With one derivation there is
/// nothing left to disagree.
///
/// Every revert below stops the chain. This runs as a system call from the
/// node's pre-execution stage, so a revert is a block-execution error on every
/// node, before any transaction in the block runs — which means no transaction
/// can repair the state afterwards. Each one is therefore an assertion of an
/// assumption held elsewhere, not a condition this contract expects to meet.
pub fn commit_epoch_committee<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    if sdk.context().contract_caller() != SYSTEM_CALLER {
        return revert(sdk, ERR_ONLY_SYSTEM_CALL);
    }
    let storage = consensus_storage();
    let current = current_epoch(sdk)?;
    let target = storage
        .last_committed_epoch_p1_accessor()
        .get_checked(sdk)?;
    if target
        > current
            .checked_add(MAX_COMMITTEE_LOOKAHEAD_EPOCHS)
            .ok_or(ExitCode::IntegerOverflow)?
    {
        return revert_with(sdk, ERR_EPOCH_NOT_YET_COMMITTABLE, &(target, current));
    }
    let selection_epoch = target.saturating_sub(MAX_COMMITTEE_LOOKAHEAD_EPOCHS);
    let mut members = selected_committee_at(sdk, selection_epoch)?;
    if members.len() < MIN_COMMITTEE_LENGTH {
        return revert_with(
            sdk,
            ERR_COMMITTEE_TOO_SMALL,
            &(U256::from(members.len()), U256::from(MIN_COMMITTEE_LENGTH)),
        );
    }
    // Peer-key ascending IS the consensus index space: `record_production`
    // credits `produced[epoch][leader_index]` and `judge` resolves that same
    // index against this array, and `leader_index` is the member's position in
    // the off-chain participant set, which is sorted on this key.
    // Producing that order here rather than checking a supplied one removes the
    // only way the two could have disagreed. Peer keys are unique
    // (`ERR_PEER_PUBKEY_ALREADY_IN_USE`), so there are no ties and an unstable
    // sort is deterministic.
    members.sort_unstable_by_key(|member| member.peer_pubkey);

    let changed = committee_changed(sdk, target, &members)?;
    // `target == 0` ALWAYS appends. `committee_changed` short-circuits to
    // `false` at genesis, which was inert while the committee write was
    // unconditional — this line is what makes `changed` decide whether a record
    // exists at all, and the `else` arm would resolve a record that was never
    // written. Repurposing a branch requires establishing what its inertness
    // rested on; here it rested on nobody caring what the genesis answer was.
    //
    // Knock-on: after this, "a record was appended" no longer implies
    // "changed == true" — epoch 0 is the counterexample. Anything asking "did
    // the committee change at E" must read `dkg_qual[E]`, never infer it from a
    // record's existence.
    let record = if changed || target == 0 {
        let stored = storage.committee_records_accessor().entry(target);
        for member in &members {
            stored
                .grow_checked(sdk)?
                .set_checked(sdk, member.validator)?;
        }
        target
    } else {
        committee_at(sdk, target - 1)?.0
    };
    let index = storage.epoch_index_accessor().entry(target);
    index.record_accessor().set_checked(sdk, record as u32)?;
    index
        .length_accessor()
        .set_checked(sdk, members.len() as u32)?;
    // The weights are the ranking already read at the SELECTION epoch — the same
    // vintage that decided membership — carried through rather than looked up
    // again, and written from this same slice in this same call. That is what
    // the alignment proof is a statement about.
    write_ring(sdk, target, &members)?;
    storage
        .dkg_qual_accessor()
        .entry(target)
        .set_checked(sdk, changed)?;
    storage
        .last_committed_epoch_p1_accessor()
        .set_checked(sdk, target.checked_add(1).ok_or(ExitCode::IntegerOverflow)?)?;
    events::EpochCommitteeCommitted {
        epoch: target,
        committee: members.iter().map(|member| member.validator).collect(),
    }
    .emit(sdk)
}

/// Public handler `0x2660899f` (`getDkgQual`).
///
/// Returns the DKG-qualified committee members for an epoch.
pub fn get_dkg_qual<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    let epoch = decode::<U64Command>(input)?.value;
    write_abi(
        sdk,
        &consensus_storage()
            .dkg_qual_accessor()
            .entry(epoch)
            .get_checked(sdk)?,
    )
}

/// The validator seated at `signer_idx` of `epoch`'s frozen committee.
///
/// The membership record is the consensus index space, so this is the only
/// mapping from a signer index to an identity — and it stays a shared helper
/// rather than an inlined walk because a second walk that disagreed would
/// resolve a verdict onto the wrong validator. It had two callers; `resolveSigner`
/// was deleted for want of any, and the system-call slash entry is the survivor.
fn committee_member_at<SDK: SharedAPI>(
    sdk: &mut SDK,
    epoch: u64,
    signer_idx: u32,
) -> Result<Address, ExitCode> {
    let (record, len) = committee_at(sdk, epoch)?;
    if len == 0 {
        return revert_with(sdk, ERR_EPOCH_COMMITTEE_NOT_COMMITTED, &epoch);
    }
    if signer_idx as u64 >= len {
        return revert_with(
            sdk,
            ERR_SIGNER_INDEX_OUT_OF_RANGE,
            &(epoch, signer_idx, U256::from(len)),
        );
    }
    consensus_storage()
        .committee_records_accessor()
        .entry(record)
        .at(signer_idx as u64)
        .get_checked(sdk)
}

fn read_committee<SDK: SharedAPI>(sdk: &SDK, epoch: u64) -> Result<Vec<Address>, ExitCode> {
    let (record, len) = committee_at(sdk, epoch)?;
    let members = consensus_storage()
        .committee_records_accessor()
        .entry(record);
    let mut result = Vec::with_capacity(len as usize);
    for index in 0..len {
        result.push(members.at(index).get_checked(sdk)?);
    }
    Ok(result)
}

/// Public handler `0x80b562de` (`getEpochCommittee`).
///
/// Returns the committed validator committee for an epoch.
pub fn get_epoch_committee<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    write_abi(
        sdk,
        &read_committee(sdk, decode::<U64Command>(input)?.value)?,
    )
}

/// Public handler `0xa4d160c1` (`getEpochCommitteeWithStakes`).
///
/// Returns an epoch committee with consensus keys, historical stakes and the
/// per-member equivocation tombstone.
///
/// The tombstone rides this snapshot rather than a view of its own because the
/// node already reads the snapshot on the path where it needs the flag; a
/// dedicated `isTombstoned(address)` would add one call per member per read.
/// Unlike the other three legs it is read LIVE rather than frozen at the commit:
/// a verdict landing mid-epoch has to be visible to the committee it names,
/// which is the whole reason for reporting it.
///
/// **Membership answers at any depth; weights do not.** Once the ring has
/// wrapped past `epoch` the `stakes` leg comes back **empty** while the other
/// three stay populated — an explicit not-retained, never a vector of zeros,
/// because a caller cannot tell zeros from a real answer. The length disagreement
/// is the signal; the node's reader is taught to read it as absence rather than
/// as the corruption it would have been when the two legs came from one vector.
pub fn get_epoch_committee_with_stakes<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    let epoch = decode::<U64Command>(input)?.value;
    let (record, len) = committee_at(sdk, epoch)?;
    let members = consensus_storage()
        .committee_records_accessor()
        .entry(record);
    let weights = read_weights(sdk, epoch)?;
    let mut validators = Vec::with_capacity(len as usize);
    let mut keys = Vec::with_capacity(len as usize);
    let mut tombstoned = Vec::with_capacity(len as usize);
    for index in 0..len {
        let validator = members.at(index).get_checked(sdk)?;
        keys.push(read_consensus_keys(sdk, validator)?);
        tombstoned.push(
            consensus_storage()
                .tombstoned_accessor()
                .entry(validator)
                .get_checked(sdk)?,
        );
        validators.push(validator);
    }
    let stakes: Vec<U256> = weights
        .unwrap_or_default()
        .into_iter()
        .map(math::expand_balance)
        .collect();
    write_returns(sdk, &(validators, keys, stakes, tombstoned))
}
// Equivocation proofs and permanent validator tombstoning.

const BLS_SIG_DST: &[u8] = b"BLS_SIG_BLS12381G1_XMD:SHA-256_SSWU_RO_POP_";

/// `kind` is an `EVIDENCE_MESSAGE_KIND_*`: the kind of one message, not of the
/// conflict. A nullify/finalize proof carries two different kinds and reaches
/// this twice with different values.
fn namespace<SDK: SharedAPI>(sdk: &SDK, kind: u8) -> Bytes {
    let mut result = b"FLUENT_DPOS_V1_".to_vec();
    result.extend_from_slice(&sdk.context().block_chain_id().to_be_bytes());
    result.extend_from_slice(match kind {
        EVIDENCE_MESSAGE_KIND_NOTARIZE => b"_NOTARIZE",
        EVIDENCE_MESSAGE_KIND_NULLIFY => b"_NULLIFY",
        _ => b"_FINALIZE",
    });
    Bytes::from(result)
}

pub(crate) fn seize_self_stake<SDK: SharedAPI>(
    sdk: &mut SDK,
    validator: Address,
    owner: Address,
) -> Result<(), ExitCode> {
    let storage = staking_storage();
    let delegation = storage
        .validator_delegations_accessor()
        .entry(validator)
        .entry(owner);
    let queue = delegation.delegate_queue_accessor();
    let len = queue.len_checked(sdk)?;
    let mut seized = U256::ZERO;
    if len != 0 {
        seized = math::expand_balance(queue.at(len - 1).amount_accessor().get_checked(sdk)?);
    }

    let undelegates = delegation.undelegate_queue_accessor();
    let pending_undelegated = delegation.pending_undelegated_accessor();
    seized = seized
        .checked_add(pending_undelegated.get_checked(sdk)?)
        .ok_or(ExitCode::IntegerOverflow)?;
    if seized.is_zero() {
        return Ok(());
    }

    // Stake on its way to the burn sink must stop counting as bonded. The
    // decrease starts at the next epoch for the same reason `undelegate_from`
    // does: earlier snapshots denominate rewards that already accrued, and the
    // committee for the current epoch is frozen. It has to run before the queue
    // below is cleared, because it reads the queue to learn what each snapshot
    // was actually counting.
    remove_delegation_from_totals(sdk, validator, owner, next_epoch(sdk)?)?;

    queue.clear_checked(sdk)?;
    delegation
        .claimed_through_epoch_accessor()
        .set_checked(sdk, 0)?;
    undelegates.clear_checked(sdk)?;
    delegation.undelegate_gap_accessor().set_checked(sdk, 0)?;
    pending_undelegated.set_checked(sdk, U256::ZERO)?;

    let configured_fund = chain_config_storage()
        .slash_fund_address_accessor()
        .get_checked(sdk)?;
    let recipient = if configured_fund.is_zero() {
        EQUIVOCATION_BURN_SINK
    } else {
        configured_fund
    };
    // A refused payout REVERTS the whole seizure, and with it the tombstone, the
    // jail and the active-set removal that were written above. The alternative
    // this replaces swallowed the refusal, emitted `seized = 0` and left the
    // stake sitting on this contract with no path off it — a silent, permanent
    // loss that no view reported and no later call could undo. Rolling back
    // keeps the two halves of the penalty together: either the validator is
    // tombstoned AND its bond moved, or the charge did not land and can be
    // brought again once the recipient accepts.
    //
    // What this costs: a token that refuses the configured fund makes
    // equivocation unslashable for as long as it refuses. That is survivable
    // where the swallowed version was not — the node soft-folds a revert from
    // `slashEquivocation` (warn, state uncommitted, `node/src/evm.rs`), so the
    // chain keeps producing and governance can point `slashFundAddress`
    // somewhere that accepts; the default recipient is a burn sink, which
    // refuses nothing.
    if !try_transfer(sdk, recipient, seized)? {
        return revert(sdk, ERR_STAKING_TOKEN_CALL_FAILED);
    }
    events::EquivocationStakeSeized {
        validator,
        seized,
        recipient,
    }
    .emit(sdk)
}

/// The terminal effects of a proven equivocation, whichever route proved it.
///
/// `conflict_epoch` is the epoch the conflict happened in; the penalty is
/// stamped at the current epoch, which is later whenever a charge lands after
/// its own epoch closed.
///
/// The status is read before the tombstone is written. The other order made the
/// `ValidatorNotFound` revert roll the tombstone back, so a charge naming an
/// unregistered address left no trace at all.
fn apply_equivocation_penalty<SDK: SharedAPI>(
    sdk: &mut SDK,
    validator: Address,
    conflict_epoch: u64,
) -> Result<(), ExitCode> {
    let record = staking_storage().validators_accessor().entry(validator);
    let status = record.status_accessor().get_checked(sdk)?;
    if status == STATUS_NOT_FOUND {
        return revert_with(sdk, ERR_VALIDATOR_NOT_FOUND, &validator);
    }
    // A verified conflict is terminal: the validator cannot re-register keys.
    consensus_storage()
        .tombstoned_accessor()
        .entry(validator)
        .set_checked(sdk, true)?;
    if status == STATUS_ACTIVE {
        remove_active(sdk, validator)?;
    }
    record.status_accessor().set_checked(sdk, STATUS_JAIL)?;
    let penalty_epoch = current_epoch(sdk)?;
    set_selection_visible(sdk, validator, false, penalty_epoch)?;
    let owner = record.owner_accessor().get_checked(sdk)?;
    seize_self_stake(sdk, validator, owner)?;
    events::ValidatorJailed {
        validator,
        epoch: penalty_epoch,
    }
    .emit(sdk)?;
    events::EquivocationSlashed {
        validator,
        epoch: conflict_epoch,
    }
    .emit(sdk)
}

/// Public handler `0xdc6fb3f2` (`slashEquivocation`).
///
/// Applies a verdict the committee already reached: position `signerIdx` of
/// `epoch` equivocated.
///
/// No evidence is carried and none is verified here. Every committee member
/// checked the charge against the block it rode in before voting for that
/// block, which is the same trust basis as any other state transition. The
/// evidence-carrying handlers below exist for the charges that outlive their
/// epoch, where no live committee can verify and the contract must.
pub fn slash_equivocation<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    if sdk.context().contract_caller() != SYSTEM_CALLER {
        return revert(sdk, ERR_ONLY_SYSTEM_CALL);
    }
    let command = decode::<EpochSignerCommand>(input)?;
    let validator = committee_member_at(sdk, command.epoch, command.signer_idx)?;
    // A duplicate is a race, not a fault: two proposers may carry the same
    // charge, and the epoch-boundary fallback transaction may land beside a
    // block-borne one. Reverting would let a system caller turn that race into a
    // failed pre-execution call.
    //
    // Not a rule about this handler, which DOES have a reachable revert further
    // down — a refused seizure rolls the whole penalty back (K-22), and the node
    // absorbs it with a warn and an uncommitted state. The difference is what the
    // two mean: a refused payout is a condition an operator repairs by rotating
    // the fund, while a duplicate charge is two honest proposers agreeing and has
    // nothing to repair.
    if consensus_storage()
        .tombstoned_accessor()
        .entry(validator)
        .get_checked(sdk)?
    {
        return Ok(());
    }
    apply_equivocation_penalty(sdk, validator, command.epoch)
}

fn slash_from_evidence<SDK: SharedAPI>(
    sdk: &mut SDK,
    command: EquivocationCommand,
    shape: EvidenceShape,
) -> Result<(), ExitCode> {
    let consensus = consensus_storage();
    let evidence = evidence::decode(sdk, &command.evidence, shape)?;
    let supplied_key = bls::compress_g2_unchecked(sdk, &command.pk_uncompressed)?;
    // Identity comes from the key, not from a historical committee seat.
    // `bls_pubkey_owner` is write-once and never released, so this answer cannot
    // be pruned out from under a late reporter. The compression is unchecked by
    // construction; what makes the binding sound is the PAIRING-routed verify
    // below, not this lookup.
    let validator = consensus
        .bls_pubkey_owner_accessor()
        .entry(keccak256(supplied_key))
        .get_checked(sdk)?;
    if validator.is_zero() {
        return revert(sdk, ERR_EQUIVOCATION_KEY_NOT_REGISTERED);
    }
    // The crate's only re-slash guard: without it the same evidence tombstones,
    // jails and seizes twice.
    if consensus
        .tombstoned_accessor()
        .entry(validator)
        .get_checked(sdk)?
    {
        return revert_with(sdk, ERR_ALREADY_SLASHED_FOR_EQUIVOCATION, &validator);
    }
    let registered_keys = read_consensus_keys(sdk, validator)?;
    if registered_keys.peer_pubkey.is_zero() {
        return revert_with(sdk, ERR_CONSENSUS_KEYS_NOT_SET, &validator);
    }
    let supplied_sig1 = bls::compress_g1_unchecked(sdk, &command.sig1_uncompressed)?;
    let supplied_sig2 = bls::compress_g1_unchecked(sdk, &command.sig2_uncompressed)?;
    if keccak256(supplied_sig1) != keccak256(&evidence.sig1)
        || keccak256(supplied_sig2) != keccak256(&evidence.sig2)
    {
        return revert(sdk, ERR_EQUIVOCATION_SIGNATURE_INVALID);
    }
    let valid1 = bls::verify(
        sdk,
        &namespace(sdk, evidence.kind1),
        &evidence.msg1,
        BLS_SIG_DST,
        &command.sig1_uncompressed,
        &command.pk_uncompressed,
    )?;
    let valid2 = bls::verify(
        sdk,
        &namespace(sdk, evidence.kind2),
        &evidence.msg2,
        BLS_SIG_DST,
        &command.sig2_uncompressed,
        &command.pk_uncompressed,
    )?;
    if !valid1 || !valid2 {
        return revert(sdk, ERR_EQUIVOCATION_SIGNATURE_INVALID);
    }
    apply_equivocation_penalty(sdk, validator, evidence.epoch)
}

/// Public handler `0xe28d2f63` (`slashEquivocationNotarize`).
///
/// Verifies conflicting notarizations and applies equivocation slashing.
pub fn slash_notarize<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    slash_from_evidence(
        sdk,
        decode_equivocation(input)?,
        EvidenceShape::ConflictingNotarize,
    )
}

/// Public handler `0xadd07a3e` (`slashEquivocationFinalize`).
///
/// Verifies conflicting finalizations and applies equivocation slashing.
pub fn slash_finalize<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    slash_from_evidence(
        sdk,
        decode_equivocation(input)?,
        EvidenceShape::ConflictingFinalize,
    )
}

/// Public handler `0xa10827e9` (`slashEquivocationNullifyFinalize`).
///
/// Verifies a nullify/finalize conflict and applies equivocation slashing.
pub fn slash_nullify_finalize<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    slash_from_evidence(
        sdk,
        decode_equivocation(input)?,
        EvidenceShape::NullifyFinalize,
    )
}

pub(crate) fn decode_equivocation(input: &[u8]) -> Result<EquivocationCommand, ExitCode> {
    let (evidence, pk_uncompressed, sig1_uncompressed, sig2_uncompressed) =
        decode_args::<(Bytes, Bytes, Bytes, Bytes)>(input)?;
    Ok(EquivocationCommand {
        evidence,
        pk_uncompressed,
        sig1_uncompressed,
        sig2_uncompressed,
    })
}
