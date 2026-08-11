//! Block-production accounting for the production-liveness tier.

use crate::{
    consts::*,
    events, math, staking,
    storage::{chain_config_storage, consensus_storage, production_liveness_storage},
    types::{
        AddressCommand, EpochSignerCommand, RecordProductionCommand, U64Command,
        ValidatorEpochCommand,
    },
    util::{
        current_epoch, current_epoch_at_block, decode, ensure_initialized, ensure_mutable,
        ensure_non_payable, revert, revert_with, write_abi,
    },
};
use alloc::{vec, vec::Vec};
use fluentbase_sdk::{
    bytes::BytesMut, codec::SolidityABI, Address, ContextReader, ExitCode, SharedAPI, U256,
};

/// Epoch at whose close `validator` is released, or `0` when not excluded.
pub(crate) fn readmit_at_epoch_of<SDK: SharedAPI>(
    sdk: &SDK,
    validator: Address,
) -> Result<u64, ExitCode> {
    production_liveness_storage()
        .validators_accessor()
        .entry(validator)
        .readmit_at_epoch_accessor()
        .get_checked(sdk)
}

fn committee_index_of<SDK: SharedAPI>(
    sdk: &SDK,
    validator: Address,
    epoch: u64,
) -> Result<Option<u32>, ExitCode> {
    let committee = consensus_storage().epoch_committees_accessor().entry(epoch);
    let len = committee.len_checked(sdk)?;
    for index in 0..len {
        if committee.at(index).get_checked(sdk)? == validator {
            return Ok(Some(index as u32));
        }
    }
    Ok(None)
}

/// Public handler `0x8244a2c2` (`recordProduction`).
///
/// Records one block's producer and, when the block crosses an epoch boundary,
/// closes the epoch that just ended.
///
/// `block_number` is an idempotency key, never an epoch tag: the epoch is
/// derived from it, because a height is deterministic over agreed state while a
/// proposer-supplied tag is not.
pub fn record_production<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    if sdk.context().contract_caller() != SYSTEM_CALLER {
        return revert(sdk, ERR_ONLY_SYSTEM_CALL);
    }
    let command = decode::<RecordProductionCommand>(input)?;
    let storage = production_liveness_storage();
    let last_processed = storage.last_processed_block_accessor().get_checked(sdk)?;
    if command.block_number <= last_processed {
        return Ok(());
    }
    // Read before the belt overwrites the height. The epoch cursor has no
    // storage of its own — it is a pure function of the recorded block, and two
    // cursors obliged to agree can disagree.
    let previous_epoch = current_epoch_at_block(sdk, last_processed)?;
    let epoch = current_epoch_at_block(sdk, command.block_number)?;
    storage
        .last_processed_block_accessor()
        .set_checked(sdk, command.block_number)?;
    // The close reads the counters of the epoch that ended; the credit below is
    // keyed by the one this block starts, so the two never touch the same key.
    // Keeping the close first is a robustness choice, not an invariant the key
    // scheme depends on.
    if epoch > previous_epoch {
        close_epoch(sdk, previous_epoch)?;
    }

    // Length only, never the committee: this runs on every block, and
    // materializing 51 members with their keys costs orders of magnitude more
    // than the whole per-block budget.
    let committee = consensus_storage().epoch_committees_accessor().entry(epoch);
    let committee_size = committee.len_checked(sdk)?;
    // Not yet committed: park the block. It is neither counted nor credited,
    // which keeps `sum(produced) == blocks_in_epoch` true by construction and
    // leaves the epoch short of the interval, i.e. tainted.
    if committee_size == 0 {
        return Ok(());
    }
    // A belt the vote already guaranteed. Reaching it means the off-chain index
    // space and the committed committee order have diverged, which no verify can
    // catch; the block is dropped and the epoch's shortfall surfaces it.
    if u64::from(command.leader_index) >= committee_size {
        return Ok(());
    }

    let credited = storage
        .produced_accessor()
        .entry(epoch)
        .entry(u32::from(command.leader_index));
    credited.set_checked(
        sdk,
        credited
            .get_checked(sdk)?
            .checked_add(1)
            .ok_or(ExitCode::IntegerOverflow)?,
    )?;
    let recorded = storage.blocks_in_epoch_accessor().entry(epoch);
    recorded.set_checked(
        sdk,
        recorded
            .get_checked(sdk)?
            .checked_add(1)
            .ok_or(ExitCode::IntegerOverflow)?,
    )?;
    let producer = committee
        .at(u64::from(command.leader_index))
        .get_checked(sdk)?;
    let record = storage.validators_accessor().entry(producer);
    let total = record.total_produced_accessor();
    total.set_checked(
        sdk,
        total
            .get_checked(sdk)?
            .checked_add(1)
            .ok_or(ExitCode::IntegerOverflow)?,
    )?;
    record
        .last_produced_epoch_p1_accessor()
        .set_checked(sdk, epoch.checked_add(1).ok_or(ExitCode::IntegerOverflow)?)
}

/// Close `epoch`: releases, verdicts, stipend.
///
/// Three legs in that order and with three different failure policies. Releases
/// first, so an expiring exclusion cannot be held hostage by the guard.
/// Verdicts second and fail-loud, because a rolled-back no-op would retry every
/// block forever with a warning as its only symptom. The stipend last and
/// tolerant, because it is the one leg where a frozen payment is preferable to
/// any chance of a frozen chain.
fn close_epoch<SDK: SharedAPI>(sdk: &mut SDK, epoch: u64) -> Result<(), ExitCode> {
    let config = chain_config_storage();
    let current = current_epoch(sdk)?;
    let disabled = config
        .production_liveness_disabled_accessor()
        .get_checked(sdk)?;

    // Unconditional on the correlation guard: tying releases to it would freeze
    // them during exactly the outage they exist for, and would make the
    // exclusion duration non-deterministic against a fixed ladder. Frozen with
    // the rest of the verdict state under the kill switch, so no exclusion
    // expires unnoticed while the tier is off.
    if !disabled {
        release_expired(sdk, current)?;
    }

    let recorded = production_liveness_storage()
        .blocks_in_epoch_accessor()
        .entry(epoch)
        .get_checked(sdk)?;
    let interval = config.epoch_block_interval_accessor().get_checked(sdk)?;

    // The taint is derived, not stored: epochs are height-defined and every
    // finalized block carries a record, so a complete epoch records exactly
    // `interval` of them. That is wider than a stored flag — it also catches an
    // executor skip and an out-of-range belt drop, both of which deflate the
    // denominator while tainting nothing.
    if u64::from(recorded) != interval {
        events::PartialEpoch {
            epoch,
            recorded,
            expected: interval as u32,
        }
        .emit(sdk)?;
    } else if !disabled {
        judge(sdk, epoch, current, recorded)?;
    }

    // An epoch that was never recorded at all is reachable and must not draw a
    // full pot for no work.
    if recorded > 0 {
        // Pin the rate before settling. `settle_up_to` walks the cursor across a
        // range, so a rate read at payment time prices every caught-up epoch at
        // today's value. Stored `+1` so "never closed" stays distinguishable
        // from "closed at a rate of zero".
        let rate = config.blend_stipend_per_epoch_accessor().get_checked(sdk)?;
        production_liveness_storage()
            .stipend_rate_at_close_p1_accessor()
            .entry(epoch)
            .set_checked(
                sdk,
                rate.checked_add(U256::ONE)
                    .ok_or(ExitCode::IntegerOverflow)?,
            )?;
        settle_stipend_leg(sdk, epoch)?;
    }
    Ok(())
}

/// Release every exclusion whose term has expired. Bounded by `f`.
fn release_expired<SDK: SharedAPI>(sdk: &mut SDK, current: u64) -> Result<(), ExitCode> {
    let storage = production_liveness_storage();
    let pending = storage.pending_exclusions_accessor();
    let mut index = 0;
    while index < pending.len_checked(sdk)? {
        let validator = pending.at(index).get_checked(sdk)?;
        let readmit = storage
            .validators_accessor()
            .entry(validator)
            .readmit_at_epoch_accessor()
            .get_checked(sdk)?;
        if readmit == 0 || readmit > current {
            index += 1;
            continue;
        }
        // The Active/tombstone guard lives inside the callee, which owns that
        // state. The record clears either way: a released-but-not-restamped
        // entry would pin a slot in the `<= f` concurrent budget forever.
        // `kick_count` survives — the ladder counts episodes across a
        // validator's whole life.
        staking::release_production_exclusion(sdk, validator)?;
        storage
            .validators_accessor()
            .entry(validator)
            .readmit_at_epoch_accessor()
            .set_checked(sdk, 0)?;
        let last = pending.len_checked(sdk)? - 1;
        if index != last {
            let tail = pending.at(last).get_checked(sdk)?;
            pending.at(index).set_checked(sdk, tail)?;
        }
        pending.pop_checked(sdk)?;
        // Swap-pop slid a new element into `index`, so do not advance.
    }
    Ok(())
}

/// The verdict sweep for a fully recorded epoch.
///
/// `due_i = w_i / W * recorded` is an expectation from on-chain stake, not a
/// replay of the leader lottery. `w_i` is the weight frozen at commit from the
/// selection epoch — the exact weight the lottery drew with — which is what
/// makes the expectation exact rather than approximate. Both predicates are
/// cross-multiplied: no division, no fixed point.
///
/// Failing below half of `due` means the member's per-slot success rate is
/// below half the stake-weighted fleet average, so uniform degradation moves
/// every member together and fails nobody at any severity, while a dead member
/// has `produced == 0` and fails at every dilution level.
fn judge<SDK: SharedAPI>(
    sdk: &mut SDK,
    epoch: u64,
    current: u64,
    recorded: u32,
) -> Result<(), ExitCode> {
    let consensus = consensus_storage();
    let committee = consensus.epoch_committees_accessor().entry(epoch);
    let member_count = committee.len_checked(sdk)?;
    if member_count == 0 {
        return Ok(());
    }
    let frozen = consensus.leader_stakes_accessor().entry(epoch);
    let frozen_count = frozen.len_checked(sdk)?;
    if frozen_count != member_count {
        return revert_with(
            sdk,
            ERR_LEADER_STAKES_LENGTH_MISMATCH,
            &(
                epoch,
                U256::from(member_count),
                U256::from(frozen_count),
            ),
        );
    }

    let mut members = Vec::with_capacity(member_count as usize);
    let mut weights = Vec::with_capacity(member_count as usize);
    let mut total_weight = U256::ZERO;
    for index in 0..member_count {
        members.push(committee.at(index).get_checked(sdk)?);
        let weight = math::expand_balance(frozen.at(index).get_checked(sdk)?);
        total_weight = total_weight
            .checked_add(weight)
            .ok_or(ExitCode::IntegerOverflow)?;
        weights.push(weight);
    }
    if total_weight.is_zero() {
        return Ok(());
    }

    let floor = U256::from(
        chain_config_storage()
            .min_verdict_due_blocks_accessor()
            .get_checked(sdk)?,
    );
    let floor_scaled = floor
        .checked_mul(total_weight)
        .ok_or(ExitCode::IntegerOverflow)?;
    let recorded_blocks = U256::from(recorded);
    let storage = production_liveness_storage();
    let mut failed = vec![false; member_count as usize];
    let mut failures = 0usize;
    let mut new_failures = 0usize;

    for (index, weight) in weights.iter().enumerate() {
        let due_scaled = weight
            .checked_mul(recorded_blocks)
            .ok_or(ExitCode::IntegerOverflow)?;
        // A zero-weight member is unjudgeable, and that is coherent — it was
        // never due a slot. Frozen weights make this a normal transient for a
        // newly seated validator, not only the signature of an abandoned seat.
        if due_scaled < floor_scaled {
            continue;
        }
        let produced = storage
            .produced_accessor()
            .entry(epoch)
            .entry(index as u32)
            .get_checked(sdk)?;
        if U256::from(produced)
            .checked_mul(U256::from(2))
            .ok_or(ExitCode::IntegerOverflow)?
            .checked_mul(total_weight)
            .ok_or(ExitCode::IntegerOverflow)?
            >= due_scaled
        {
            continue;
        }

        failed[index] = true;
        failures += 1;
        let validator = members[index];
        events::ProductionVerdictFailed {
            epoch,
            validator,
            produced,
            due: due_scaled / total_weight,
        }
        .emit(sdk)?;

        // Counted as new only if the member did not also fail the previous epoch
        // and is not already being answered. A chronic failer is evidence of
        // itself, not of an environment; without that test `f` colluders plus
        // one donated honest failure would hold the guard on forever.
        //
        // `last_failed_epoch_p1` stores epoch+1, so "failed E−1" is `p1 ==
        // epoch` — but only when `p1` is a real record. At epoch 0 the
        // never-failed sentinel collides with the encoding of a nonexistent
        // epoch −1, and a bare inequality would read a first-ever failure as a
        // repeat.
        let record = storage.validators_accessor().entry(validator);
        let previous_p1 = record.last_failed_epoch_p1_accessor().get_checked(sdk)?;
        let failed_previous_epoch = previous_p1 != 0 && previous_p1 == epoch;
        if !failed_previous_epoch && record.readmit_at_epoch_accessor().get_checked(sdk)? == 0 {
            new_failures += 1;
        }
    }

    // Written for every failer on both paths, and only after the newness test
    // has read the old values. Writing it just when the guard fires would leave
    // a chronic failer's bit stale, so it would pass the newness test forever.
    let stamp_epoch_p1 = epoch.checked_add(1).ok_or(ExitCode::IntegerOverflow)?;
    for (index, did_fail) in failed.iter().enumerate() {
        if *did_fail {
            storage
                .validators_accessor()
                .entry(members[index])
                .last_failed_epoch_p1_accessor()
                .set_checked(sdk, stamp_epoch_p1)?;
        }
    }

    let tolerance = math::fault_tolerance(member_count as usize);
    if new_failures > tolerance {
        // An environment breaks suddenly, so it shows up as a jump in first-time
        // failures. One epoch of amnesty, not an open-ended shield: from the
        // second epoch its members are no longer new.
        events::CorrelatedFailureEpoch {
            epoch,
            new_failures: U256::from(new_failures),
            tolerance: U256::from(tolerance),
        }
        .emit(sdk)?;
        return Ok(());
    }
    if failures == 0 {
        return Ok(());
    }
    stamp(sdk, current, &members, &mut failed, tolerance)
}

/// At most `MAX_STAMPS_PER_CLOSE` stamps per close, never more than `f`
/// concurrent.
///
/// The order is kick-count descending then address ascending — stated rather
/// than left to storage iteration order, which is deterministic but is an
/// accident a future layout change could silently alter with no spec to violate.
fn stamp<SDK: SharedAPI>(
    sdk: &mut SDK,
    current: u64,
    members: &[Address],
    failed: &mut [bool],
    tolerance: usize,
) -> Result<(), ExitCode> {
    let cap = chain_config_storage()
        .exclusion_backoff_cap_accessor()
        .get_checked(sdk)?;
    let storage = production_liveness_storage();
    let pending = storage.pending_exclusions_accessor();
    for _ in 0..MAX_STAMPS_PER_CLOSE {
        if pending.len_checked(sdk)? as usize >= tolerance {
            break;
        }
        // Two-pass max-selection rather than a sort: only two picks are needed.
        let mut best: Option<usize> = None;
        for (index, did_fail) in failed.iter().enumerate() {
            if !*did_fail {
                continue;
            }
            let record = storage.validators_accessor().entry(members[index]);
            if record.readmit_at_epoch_accessor().get_checked(sdk)? != 0 {
                continue;
            }
            let Some(leader) = best else {
                best = Some(index);
                continue;
            };
            let leader_kicks = storage
                .validators_accessor()
                .entry(members[leader])
                .kick_count_accessor()
                .get_checked(sdk)?;
            let kicks = record.kick_count_accessor().get_checked(sdk)?;
            if kicks > leader_kicks || (kicks == leader_kicks && members[index] < members[leader]) {
                best = Some(index);
            }
        }
        let Some(best) = best else {
            break;
        };
        // Consumed either way, so a refusal cannot spin.
        failed[best] = false;
        // A refused stamp must leave no trace: no ladder increment, no queue
        // entry. Otherwise the backoff advances for an exclusion that never was.
        if !staking::apply_production_exclusion(sdk, members[best])? {
            continue;
        }
        let record = storage.validators_accessor().entry(members[best]);
        let episodes = record
            .kick_count_accessor()
            .get_checked(sdk)?
            .checked_add(1)
            .ok_or(ExitCode::IntegerOverflow)?;
        record.kick_count_accessor().set_checked(sdk, episodes)?;
        // Measured from the current epoch, not the judged one. The two coincide
        // when the close runs on the first block of the next epoch, but a close
        // can run late, and the current-epoch form keeps the ladder length
        // intact instead of silently shortening the exclusion.
        let duration = u64::from(core::cmp::min(episodes, cap));
        record.readmit_at_epoch_accessor().set_checked(
            sdk,
            current
                .checked_add(duration)
                .ok_or(ExitCode::IntegerOverflow)?,
        )?;
        pending.push_checked(sdk, members[best])?;
    }
    Ok(())
}

/// Run the stipend inside a fuel-capped self-call.
///
/// The host builds a real frame with its own journal checkpoint, so a revert
/// inside it discards only that frame's writes — every release and verdict
/// above has already landed and survives. Failure arrives as a status and never
/// as an unwind: `unwrap` on the result would abort the outer frame under
/// `panic = "abort"`, which is the exact opposite of tolerance. `OutOfFuel` and
/// a revert are distinguishable and both are tolerated.
///
/// The event is emitted from this frame deliberately: a log written inside the
/// discarded frame goes with it, and a system call leaves no receipt to read the
/// failure from instead.
fn settle_stipend_leg<SDK: SharedAPI>(sdk: &mut SDK, epoch: u64) -> Result<(), ExitCode> {
    let mut params = BytesMut::new();
    SolidityABI::<U64Command>::encode(&U64Command { value: epoch }, &mut params, 0)
        .map_err(|_| ExitCode::MalformedBuiltinParams)?;
    let mut input = SIG_SETTLE_EPOCH_STIPEND_FROM.to_be_bytes().to_vec();
    input.extend_from_slice(&params);
    let own_address = sdk.context().contract_address();
    let result = sdk.call(own_address, U256::ZERO, &input, Some(STIPEND_FUEL_CAP));
    if !result.status.is_ok() {
        events::StipendLegSkipped { epoch }.emit(sdk)?;
    }
    Ok(())
}

/// Public handler `0x8e948ac1` (`getProductionStats`).
///
/// Returns the delegator-facing production record for `validator` at `epoch`.
pub fn get_production_stats<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    // Reads the committee, so it follows the consensus readers rather than the
    // config getters: all-zeros must not be indistinguishable from uninitialized.
    ensure_initialized(sdk)?;
    let command = decode::<ValidatorEpochCommand>(input)?;
    let epoch = command.before_epoch;
    let storage = production_liveness_storage();
    let record = storage.validators_accessor().entry(command.validator);
    let produced_this_epoch = match committee_index_of(sdk, command.validator, epoch)? {
        Some(index) => storage
            .produced_accessor()
            .entry(epoch)
            .entry(index)
            .get_checked(sdk)?,
        None => 0,
    };
    let result = (
        produced_this_epoch,
        record.total_produced_accessor().get_checked(sdk)?,
        record
            .last_produced_epoch_p1_accessor()
            .get_checked(sdk)?
            .saturating_sub(1),
        record
            .last_failed_epoch_p1_accessor()
            .get_checked(sdk)?
            .saturating_sub(1),
        record.kick_count_accessor().get_checked(sdk)?,
        record.readmit_at_epoch_accessor().get_checked(sdk)?,
    );
    write_abi(sdk, &result)
}

/// Public handler `0xf06be669` (`blocksInEpoch`).
///
/// Returns the number of blocks recorded for `epoch`.
pub fn blocks_in_epoch<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let epoch = decode::<U64Command>(input)?.value;
    write_abi(
        sdk,
        &production_liveness_storage()
            .blocks_in_epoch_accessor()
            .entry(epoch)
            .get_checked(sdk)?,
    )
}

/// Public handler `0x91c7d453` (`producedAt`).
///
/// Returns the blocks credited to a committee index in `epoch`.
pub fn produced_at<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let command = decode::<EpochSignerCommand>(input)?;
    write_abi(
        sdk,
        &production_liveness_storage()
            .produced_accessor()
            .entry(command.epoch)
            .entry(command.signer_idx)
            .get_checked(sdk)?,
    )
}

/// Public handler `0xaef690f9` (`pendingExclusions`).
///
/// Returns the validators currently serving an exclusion.
pub fn pending_exclusions<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let entries = production_liveness_storage().pending_exclusions_accessor();
    let len = entries.len_checked(sdk)?;
    let mut result = Vec::with_capacity(len as usize);
    for index in 0..len {
        result.push(entries.at(index).get_checked(sdk)?);
    }
    write_abi(sdk, &result)
}

/// Public handler `0x32066046` (`readmitAtEpoch`).
///
/// Returns the epoch at whose close `validator` is released from exclusion.
pub fn readmit_at_epoch<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let validator = decode::<AddressCommand>(input)?.value;
    write_abi(sdk, &readmit_at_epoch_of(sdk, validator)?)
}

/// Public handler `0x33de61d2` (`lastProcessedBlock`).
///
/// Returns the most recent block for which production was recorded.
pub fn last_processed_block<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(
        sdk,
        &production_liveness_storage()
            .last_processed_block_accessor()
            .get_checked(sdk)?,
    )
}
