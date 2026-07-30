//! Authorized liveness slashing, jail transitions, and bounded readmission.

use crate::{
    consts::*,
    events,
    storage::{
        current_epoch, remove_active, staking_storage, STATUS_ACTIVE, STATUS_JAIL, STATUS_NOT_FOUND,
    },
    types::{AddressCommand, U64Command},
    util::{
        decode, ensure_initialized, ensure_mutable, ensure_non_payable, revert, revert_with,
        set_selection_visible, touch_snapshot_at_or_before,
    },
};
use fluentbase_sdk::{Address, ContextReader, ExitCode, SharedAPI, U256};

fn ensure_liveness<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    let caller = sdk.context().contract_caller();
    let expected = staking_storage()
        .config_accessor()
        .liveness_slashing_accessor()
        .get_checked(sdk)?;
    if expected.is_zero() || caller != expected {
        return revert(sdk, ERR_ONLY_LIVENESS_SLASHING);
    }
    Ok(())
}

pub(crate) fn remove_jailed<SDK: SharedAPI>(
    sdk: &mut SDK,
    validator: Address,
) -> Result<(), ExitCode> {
    let jailed = staking_storage().jailed_validators_accessor();
    let len = jailed.len_checked(sdk)?;
    for index in 0..len {
        if jailed.at(index).get_checked(sdk)? != validator {
            continue;
        }
        if index + 1 != len {
            let last = jailed.at(len - 1).get_checked(sdk)?;
            jailed.at(index).set_checked(sdk, last)?;
        }
        jailed.pop_checked(sdk)?;
        break;
    }
    Ok(())
}

fn readmit<SDK: SharedAPI>(sdk: &mut SDK, validator: Address) -> Result<(), ExitCode> {
    let storage = staking_storage();
    storage
        .validators_accessor()
        .entry(validator)
        .status_accessor()
        .set_checked(sdk, STATUS_ACTIVE)?;
    storage
        .active_validators_accessor()
        .push_checked(sdk, validator)?;
    remove_jailed(sdk, validator)?;
    let epoch = current_epoch(sdk)?;
    set_selection_visible(sdk, validator, true, epoch)?;
    events::ValidatorReleased { validator, epoch }.emit(sdk)
}

pub fn release_validator_from_jail<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    let validator = decode::<AddressCommand>(input)?.value;
    let storage = staking_storage();
    if storage
        .tombstoned_accessor()
        .entry(validator)
        .get_checked(sdk)?
    {
        return revert_with(sdk, ERR_ALREADY_SLASHED_FOR_EQUIVOCATION, &validator);
    }
    let record = storage.validators_accessor().entry(validator);
    if record.status_accessor().get_checked(sdk)? != STATUS_JAIL {
        return revert_with(sdk, ERR_VALIDATOR_NOT_IN_JAIL, &validator);
    }
    let owner = record.owner_accessor().get_checked(sdk)?;
    if sdk.context().contract_caller() != owner {
        return revert_with(sdk, ERR_ONLY_VALIDATOR_OWNER, &owner);
    }
    if current_epoch(sdk)? < record.jailed_before_accessor().get_checked(sdk)? {
        return revert_with(sdk, ERR_STILL_IN_JAIL, &validator);
    }
    readmit(sdk, validator)
}

pub fn readmit_expired_jails<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_liveness(sdk)?;
    let _: U64Command = decode(input)?;
    let epoch = current_epoch(sdk)?;
    let storage = staking_storage();
    let jailed = storage.jailed_validators_accessor();
    let len = jailed.len_checked(sdk)?;
    if len == 0 {
        storage.jailed_scan_cursor_accessor().set_checked(sdk, 0)?;
        return Ok(());
    }
    let mut index = storage.jailed_scan_cursor_accessor().get_checked(sdk)? % len;
    let mut examined = 0;
    // Persist the cursor so each call does bounded work without starving entries.
    let budget = core::cmp::min(len, MAX_ACTIVE_VALIDATORS_LENGTH);
    while examined < budget {
        let current_len = jailed.len_checked(sdk)?;
        if current_len == 0 {
            index = 0;
            break;
        }
        if index >= current_len {
            index = 0;
        }
        let validator = jailed.at(index).get_checked(sdk)?;
        let record = storage.validators_accessor().entry(validator);
        if storage
            .tombstoned_accessor()
            .entry(validator)
            .get_checked(sdk)?
        {
            remove_jailed(sdk, validator)?;
        } else if record.status_accessor().get_checked(sdk)? == STATUS_JAIL
            && epoch >= record.jailed_before_accessor().get_checked(sdk)?
        {
            readmit(sdk, validator)?;
        } else {
            index = (index + 1) % current_len;
        }
        examined += 1;
    }
    let remaining = jailed.len_checked(sdk)?;
    storage
        .jailed_scan_cursor_accessor()
        .set_checked(sdk, if remaining == 0 { 0 } else { index % remaining })
}

fn quorum(n: u64) -> u64 {
    if n == 0 {
        return 0;
    }
    n - (n - 1) / 3
}

pub fn slash<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_liveness(sdk)?;
    let validator = decode::<AddressCommand>(input)?.value;
    let storage = staking_storage();
    let record = storage.validators_accessor().entry(validator);
    let status = record.status_accessor().get_checked(sdk)?;
    if status == STATUS_NOT_FOUND {
        return revert_with(sdk, ERR_VALIDATOR_NOT_FOUND, &validator);
    }
    let epoch = current_epoch(sdk)?;
    let snapshot = touch_snapshot_at_or_before(sdk, validator, epoch)?;
    let slashes = snapshot
        .slashes_count_accessor()
        .get_checked(sdk)?
        .checked_add(1)
        .ok_or(ExitCode::IntegerOverflow)?;
    snapshot
        .slashes_count_accessor()
        .set_checked(sdk, slashes)?;

    let threshold = storage
        .config_accessor()
        .felony_threshold_accessor()
        .get_checked(sdk)?;
    if slashes >= threshold {
        let active_len = storage.active_validators_accessor().len_checked(sdk)?;
        let cap = storage
            .config_accessor()
            .active_validators_length_accessor()
            .get_checked(sdk)?;
        let quorum_floor = quorum(core::cmp::min(active_len, cap));
        if active_len == 0 || active_len - 1 < quorum_floor {
            events::LivenessJailSkippedHaltGuard {
                validator,
                epoch,
                active_set_size: U256::from(active_len),
                quorum_floor: U256::from(quorum_floor),
            }
            .emit(sdk)?;
        } else {
            let jail_until = epoch
                .checked_add(
                    storage
                        .config_accessor()
                        .validator_jail_epoch_length_accessor()
                        .get_checked(sdk)? as u64,
                )
                .ok_or(ExitCode::IntegerOverflow)?;
            if status != STATUS_JAIL {
                record.status_accessor().set_checked(sdk, STATUS_JAIL)?;
                record
                    .jailed_before_accessor()
                    .set_checked(sdk, jail_until)?;
                remove_active(sdk, validator)?;
                storage
                    .jailed_validators_accessor()
                    .push_checked(sdk, validator)?;
                set_selection_visible(sdk, validator, false, epoch)?;
            } else {
                record
                    .jailed_before_accessor()
                    .set_checked(sdk, jail_until)?;
            }
            events::ValidatorJailed { validator, epoch }.emit(sdk)?;
        }
    }
    events::ValidatorSlashed {
        validator,
        slashes,
        epoch,
    }
    .emit(sdk)
}
