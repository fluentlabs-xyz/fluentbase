//! Block-production accounting for the production-liveness tier.

use crate::{
    consensus,
    consts::*,
    events, math, staking,
    storage::{chain_config_storage, consensus_storage, production_liveness_storage},
    types::{RecordProductionCommand, U64Command},
    util::{
        current_epoch, current_epoch_at_block, decode, ensure_initialized, ensure_mutable,
        ensure_non_payable, revert,
    },
};
#[cfg(feature = "devnet-views")]
use crate::{types::EpochSignerCommand, util::write_abi};
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

/// Public handler `0x1752910e` (`recordProduction`).
///
/// Records one block's producer and, when the block crosses an epoch boundary,
/// closes the epoch that just ended.
///
/// The height is read from the block context, never supplied. It is the same
/// value the node would have passed, and a height the contract derives cannot
/// disagree with the block it is executing in. It serves as both the idempotency
/// key and the epoch cursor, because a height is deterministic over agreed state
/// while a proposer-supplied tag is not.
///
/// `leader_index` is the asymmetric half: it cannot be derived on-chain, which
/// is why it is verified at vote time instead.
pub fn record_production<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    if sdk.context().contract_caller() != SYSTEM_CALLER {
        return revert(sdk, ERR_ONLY_SYSTEM_CALL);
    }
    let command = decode::<RecordProductionCommand>(input)?;
    let storage = production_liveness_storage();
    let block_number = sdk.context().block_number();
    let last_processed = storage.last_processed_block_accessor().get_checked(sdk)?;
    if block_number <= last_processed {
        return Ok(());
    }
    // Read before the belt overwrites the height. The epoch cursor has no
    // storage of its own — it is a pure function of the recorded block, and two
    // cursors obliged to agree can disagree.
    let previous_epoch = current_epoch_at_block(sdk, last_processed)?;
    let epoch = current_epoch(sdk)?;
    storage
        .last_processed_block_accessor()
        .set_checked(sdk, block_number)?;
    // The close reads the counters of the epoch that ended; the credit below is
    // keyed by the one this block starts, so the two never touch the same key.
    // Keeping the close first is a robustness choice, not an invariant the key
    // scheme depends on.
    if epoch > previous_epoch {
        close_epoch(sdk, previous_epoch)?;
    }

    // Length only, never the committee: this runs on every block, and
    // materializing 51 members with their keys costs orders of magnitude more
    // than the whole per-block budget. The length lives in the index rather than
    // being read off the record for exactly that reason — and `committee_length_at`
    // rather than `committee_at`, because the two halves share a slot but not a
    // read and the record pointer is not wanted here.
    let committee_size = consensus::committee_length_at(sdk, epoch)?;
    // Not yet committed: park the block. It is neither counted nor credited,
    // which keeps `sum(produced) == blocks_in_epoch` true by construction and
    // leaves the epoch short of its expectation, i.e. tainted.
    //
    // Both park arms are deliberately silent. A system call's logs never reach a
    // receipt — the node hands them to its close-observability hook and commits
    // only the state — so an event here would reach nobody without a matching
    // node change. The epoch's shortfall is the whole trace a parked block
    // leaves, and `PartialEpoch` already reports its magnitude.
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
    )
}

/// Close `epoch`: releases, verdicts, stipend.
///
/// Three legs in that order and with three different failure policies. Releases
/// first and unconditionally, so an expiring exclusion cannot be held hostage by
/// the correlation guard or by the kill switch.
/// Verdicts second and fail-loud, because a rolled-back no-op would retry every
/// block forever with a warning as its only symptom. The stipend last and
/// tolerant, because it is the one leg where a frozen payment is preferable to
/// any chance of a frozen chain.
fn close_epoch<SDK: SharedAPI>(sdk: &mut SDK, epoch: u64) -> Result<(), ExitCode> {
    let config = chain_config_storage();
    let current = current_epoch(sdk)?;

    // Unconditional on both the correlation guard and the kill switch: tying
    // releases to either would freeze them during exactly the outage they exist
    // for, and would make the exclusion duration non-deterministic against a
    // fixed ladder. The switch exists to stop the tier punishing, and a release
    // is not a punishment — it only undoes a stamp the tier itself issued, and
    // the Active/tombstone guard inside the callee still applies. Gating it
    // would freeze the stamp, not the clock: `readmit_at_epoch` would sit still
    // while `current` ran past it, silently lengthening the exclusion, and
    // `activate_validator` refuses to rescue a validator that still carries an
    // outstanding stamp. So an exclusion cannot outlive its term while the tier
    // is off; only judging is suspended.
    release_expired(sdk, current)?;

    let recorded = production_liveness_storage()
        .blocks_in_epoch_accessor()
        .entry(epoch)
        .get_checked(sdk)?;
    let interval = config.epoch_block_interval_accessor().get_checked(sdk)?;
    // Epoch 0 has one fewer recordable height than every later epoch: the
    // activation block was produced by the pre-DPoS sequencer, which holds no
    // committee position, so its header carries empty `extra_data` and the node
    // issues no record for it (node `evm.rs`, `len == 0 => no syscall`). Expecting
    // `interval` there would taint a healthy epoch 0 on every chain and fire
    // `PartialEpoch` on schedule — an anomaly signal that always fires teaches
    // operators to stop reading it. `saturating_sub` is free insurance only: a
    // zero interval cannot reach here, because deriving the epoch fails first.
    let expected = if epoch == 0 {
        interval.saturating_sub(1)
    } else {
        interval
    };

    // The taint is derived, not stored: epochs are height-defined and every
    // recordable block carries a record, so a complete epoch records exactly
    // `expected` of them. That is wider than a stored flag — it also catches an
    // executor skip and an out-of-range belt drop, both of which deflate the
    // denominator while tainting nothing.
    if u64::from(recorded) != expected {
        events::PartialEpoch {
            epoch,
            recorded,
            expected: expected as u32,
        }
        .emit(sdk)?;
    } else if !config
        .production_liveness_disabled_accessor()
        .get_checked(sdk)?
    {
        // The one leg the kill switch holds: no new verdicts, no new stamps.
        judge(sdk, epoch, current, recorded)?;
    }

    // Unconditional, and above the leg that spends it. `close_epoch` only ever
    // runs for the epoch of the last recorded block, so an epoch it skips here
    // is an epoch nothing will ever accrue for — and the payment cursor, which
    // walks contiguously, would sit in front of it forever waiting.
    //
    // In the main frame, not inside the leg: the leg's failure is swallowed as a
    // status, which the payment survives because the cursor retries it, and an
    // accrual would not.
    staking::accrue_epoch(sdk, epoch, recorded)?;
    // No longer gated on this epoch having produced. Payment is a scalar read
    // and a transfer now, so holding the cursor back across a run of empty
    // epochs buys nothing and leaves it a backlog to recover through at
    // `MAX_SETTLE_CATCHUP` per close.
    settle_stipend_leg(sdk, epoch)
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
    let (record, member_count) = consensus::committee_at(sdk, epoch)?;
    if member_count == 0 {
        return Ok(());
    }
    // Ring miss: forfeit this epoch's verdicts and announce it. Not an error —
    // the close is a pre-execution system call, so a propagated one is a chain
    // halt no transaction can repair. And not a silent skip: the two arms around
    // this one are silent, which is exactly why a miss must not look like them.
    let Some(frozen) = consensus::read_weights(sdk, epoch)? else {
        events::EpochWeightsUnavailable {
            epoch,
            members: member_count as u32,
        }
        .emit(sdk)?;
        return Ok(());
    };
    let seated = consensus_storage()
        .committee_records_accessor()
        .entry(record);
    let mut members = Vec::with_capacity(member_count as usize);
    let mut weights = Vec::with_capacity(member_count as usize);
    let mut total_weight = U256::ZERO;
    for (index, frozen_weight) in frozen.into_iter().enumerate() {
        members.push(seated.at(index as u64).get_checked(sdk)?);
        let weight = math::expand_balance(frozen_weight);
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
            .checked_mul(U256::from(MIN_PRODUCTION_SHARE_DENOMINATOR))
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
///
/// What the discarded frame takes with it is the *payment* and nothing else. The
/// accrual ran above this call, so every epoch the leg failed to fund is still
/// on the ledger and still owed; `StipendLegSkipped` reports a deferral, not a
/// loss.
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

/// Public handler `0xf06be669` (`blocksInEpoch`).
///
/// Returns the number of blocks recorded for `epoch`.
///
/// This and the three views below it have no production consumer — `e2e/` and
/// the node repo's devnet smoke/soak harness are the only callers — so they are
/// compiled out unless `devnet-views` is on. The rest of the contract's read
/// surface is unaffected; it is these four that nothing in production serves.
#[cfg(feature = "devnet-views")]
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
#[cfg(feature = "devnet-views")]
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
#[cfg(feature = "devnet-views")]
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

/// Public handler `0x33de61d2` (`lastProcessedBlock`).
///
/// Returns the most recent block for which production was recorded.
#[cfg(feature = "devnet-views")]
pub fn last_processed_block<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(
        sdk,
        &production_liveness_storage()
            .last_processed_block_accessor()
            .get_checked(sdk)?,
    )
}
