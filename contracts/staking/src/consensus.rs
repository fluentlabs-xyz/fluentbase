//! Consensus-key registration and deterministic epoch committee commits.

use crate::{
    consts::*,
    events,
    storage::{current_epoch, staking_storage, STATUS_NOT_FOUND},
    types::{
        AddressCommand, ConsensusKeys, EpochSignerCommand, SetConsensusKeysCommand, U64Command,
    },
    util::{
        decode, decode_args, encode_external_call, ensure_initialized, ensure_mutable,
        ensure_non_payable, next_epoch, revert, revert_with, selected_validators,
        selected_validators_at, selection_visible_at, validator_total_at, write_abi, write_returns,
    },
};
use alloc::vec::Vec;
use fluentbase_sdk::{
    codec::SolidityABI, Address, Bytes, ContextReader, ExitCode, SharedAPI, B256, U256,
};

const BLS_POP_DST: &[u8] = b"BLS_POP_BLS12381G1_XMD:SHA-256_SSWU_RO_POP_";

fn external_call<SDK, T>(
    sdk: &mut SDK,
    target: Address,
    selector: u32,
    params: &T,
) -> Result<Bytes, ExitCode>
where
    SDK: SharedAPI,
    T: fluentbase_sdk::codec::FunctionArgs<fluentbase_sdk::byteorder::BE, 32, true, false>,
{
    let input = encode_external_call(selector, params)?;
    let result = sdk.call(target, U256::ZERO, &input, None);
    if !result.status.is_ok() {
        sdk.write(result.data);
        return Err(result.status);
    }
    Ok(result.data)
}

fn read_consensus_keys<SDK: SharedAPI>(
    sdk: &SDK,
    validator: Address,
) -> Result<ConsensusKeys, ExitCode> {
    let keys = staking_storage().consensus_keys_accessor().entry(validator);
    Ok(ConsensusKeys {
        bls_pubkey: Bytes::from(keys.bls_pubkey_accessor().load(sdk)?),
        peer_pubkey: keys.peer_pubkey_accessor().get_checked(sdk)?,
        activation_epoch: keys.activation_epoch_accessor().get_checked(sdk)?,
    })
}

fn fluent_namespace<SDK: SharedAPI>(sdk: &SDK) -> Bytes {
    let mut namespace = b"FLUENT_DPOS_V1_".to_vec();
    namespace.extend_from_slice(&sdk.context().block_chain_id().to_be_bytes());
    Bytes::from(namespace)
}

pub fn set_consensus_keys<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    let (validator, bls_pubkey_uncompressed, bls_pop_uncompressed, peer_pubkey) =
        decode_args::<(Address, Bytes, Bytes, B256)>(input)?;
    let command = SetConsensusKeysCommand {
        validator,
        bls_pubkey_uncompressed,
        bls_pop_uncompressed,
        peer_pubkey,
    };
    let storage = staking_storage();
    if storage
        .tombstoned_accessor()
        .entry(command.validator)
        .get_checked(sdk)?
    {
        return revert_with(
            sdk,
            ERR_ALREADY_SLASHED_FOR_EQUIVOCATION,
            &command.validator,
        );
    }
    let record = storage.validators_accessor().entry(command.validator);
    if record.status_accessor().get_checked(sdk)? == STATUS_NOT_FOUND {
        return revert_with(sdk, ERR_VALIDATOR_NOT_FOUND, &command.validator);
    }
    let owner = record.owner_accessor().get_checked(sdk)?;
    if sdk.context().contract_caller() != owner {
        return revert_with(sdk, ERR_ONLY_VALIDATOR_OWNER, &owner);
    }
    if command.bls_pubkey_uncompressed.len() != BLS_PUBKEY_UNCOMPRESSED_LENGTH
        || command.bls_pop_uncompressed.len() != BLS_POP_UNCOMPRESSED_LENGTH
        || command.peer_pubkey.is_zero()
    {
        return revert(sdk, ERR_INVALID_CONSENSUS_KEY_ENCODING);
    }
    let keys = storage.consensus_keys_accessor().entry(command.validator);
    if keys.bls_pubkey_accessor().len(sdk) != 0 {
        return revert_with(sdk, ERR_CONSENSUS_KEYS_ALREADY_SET, &command.validator);
    }
    if !storage
        .peer_pubkey_owner_accessor()
        .entry(command.peer_pubkey)
        .get_checked(sdk)?
        .is_zero()
    {
        return revert_with(sdk, ERR_PEER_PUBKEY_ALREADY_IN_USE, &command.peer_pubkey);
    }

    let verifier = storage
        .config_accessor()
        .bls_verifier_accessor()
        .get_checked(sdk)?;
    if verifier.is_zero() {
        return revert(sdk, ERR_BLS_VERIFIER_NOT_CONFIGURED);
    }
    let compressed_output = external_call(
        sdk,
        verifier,
        SIG_BLS_COMPRESS_G2_UNCHECKED,
        &(command.bls_pubkey_uncompressed.clone(),),
    )?;
    let compressed = SolidityABI::<Bytes>::decode(&compressed_output, 0)
        .map_err(|_| ExitCode::MalformedBuiltinParams)?;
    let verify_output = external_call(
        sdk,
        verifier,
        SIG_BLS_VERIFY,
        &(
            fluent_namespace(sdk),
            compressed.clone(),
            Bytes::from_static(BLS_POP_DST),
            command.bls_pop_uncompressed,
            command.bls_pubkey_uncompressed,
        ),
    )?;
    let valid = SolidityABI::<bool>::decode(&verify_output, 0)
        .map_err(|_| ExitCode::MalformedBuiltinParams)?;
    if !valid {
        return revert_with(sdk, ERR_INVALID_PROOF_OF_POSSESSION, &command.validator);
    }

    let activation_epoch = if selection_visible_at(sdk, command.validator, 0)? {
        0
    } else {
        next_epoch(sdk)?
    };
    keys.bls_pubkey_accessor().store(sdk, compressed.as_ref())?;
    keys.peer_pubkey_accessor()
        .set_checked(sdk, command.peer_pubkey)?;
    keys.activation_epoch_accessor()
        .set_checked(sdk, activation_epoch)?;
    storage
        .peer_pubkey_owner_accessor()
        .entry(command.peer_pubkey)
        .set_checked(sdk, command.validator)?;
    events::ConsensusKeysSet {
        validator: command.validator,
        bls_pubkey: compressed,
        peer_pubkey: command.peer_pubkey,
        activation_epoch,
    }
    .emit(sdk)
}

pub fn get_consensus_keys<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    let keys = read_consensus_keys(sdk, decode::<AddressCommand>(input)?.value)?;
    write_abi(sdk, &keys)
}

fn write_validators_with_keys<SDK: SharedAPI>(
    sdk: &mut SDK,
    validators: Vec<Address>,
    visible_at: Option<u64>,
) -> Result<(), ExitCode> {
    let mut keys = Vec::with_capacity(validators.len());
    for validator in &validators {
        let mut value = read_consensus_keys(sdk, *validator)?;
        if visible_at.is_some_and(|epoch| value.activation_epoch > epoch) {
            value = ConsensusKeys::default();
        }
        keys.push(value);
    }
    write_returns(sdk, &(validators, keys))
}

pub fn get_validators_with_keys<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    write_validators_with_keys(sdk, selected_validators(sdk)?, None)
}

pub fn get_registry_with_keys<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    let active = staking_storage().active_validators_accessor();
    let len = active.len_checked(sdk)?;
    let mut validators = Vec::with_capacity(len as usize);
    for index in 0..len {
        validators.push(active.at(index).get_checked(sdk)?);
    }
    write_validators_with_keys(sdk, validators, None)
}

pub fn get_validators_with_keys_at<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    let epoch = decode::<U64Command>(input)?.value;
    write_validators_with_keys(sdk, selected_validators_at(sdk, epoch)?, Some(epoch))
}

pub fn next_epoch_to_commit<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    write_abi(
        sdk,
        &staking_storage()
            .last_committed_epoch_p1_accessor()
            .get_checked(sdk)?,
    )
}

pub fn committee_selection_epoch<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    let target = staking_storage()
        .last_committed_epoch_p1_accessor()
        .get_checked(sdk)?;
    write_abi(sdk, &target.saturating_sub(2))
}

fn committee_changed<SDK: SharedAPI>(
    sdk: &SDK,
    target: u64,
    submitted: &[Address],
) -> Result<bool, ExitCode> {
    if target == 0 {
        return Ok(false);
    }
    let incumbent = staking_storage()
        .epoch_committees_accessor()
        .entry(target - 1);
    if incumbent.len_checked(sdk)? as usize != submitted.len() {
        return Ok(true);
    }
    for (index, member) in submitted.iter().enumerate() {
        if incumbent.at(index as u64).get_checked(sdk)? != *member {
            return Ok(true);
        }
    }
    Ok(false)
}

fn prune_committees<SDK: SharedAPI>(sdk: &mut SDK, current: u64) -> Result<(), ExitCode> {
    let storage = staking_storage();
    let retention = storage
        .config_accessor()
        .undelegate_period_accessor()
        .get_checked(sdk)?
        .checked_add(EPOCH_COMMITTEE_RETENTION_MARGIN)
        .ok_or(ExitCode::IntegerOverflow)?;
    if current <= retention {
        return Ok(());
    }
    let prune_to = current - retention - 1;
    let mut cursor = storage.pruned_up_to_p1_accessor().get_checked(sdk)?;
    let mut deleted = 0;
    // Bound cleanup so a long-idle chain cannot make one system call unbounded.
    while cursor <= prune_to && deleted < 16 {
        storage
            .epoch_committees_accessor()
            .entry(cursor)
            .clear_checked(sdk)?;
        cursor += 1;
        deleted += 1;
    }
    storage.pruned_up_to_p1_accessor().set_checked(sdk, cursor)
}

pub fn commit_epoch_committee<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    if sdk.context().contract_caller() != SYSTEM_CALLER {
        return revert(sdk, ERR_ONLY_SYSTEM_CALL);
    }
    let (submitted,) = decode_args::<(Vec<Address>,)>(input)?;
    let storage = staking_storage();
    let current = current_epoch(sdk)?;
    let target = storage
        .last_committed_epoch_p1_accessor()
        .get_checked(sdk)?;
    if target > current.checked_add(2).ok_or(ExitCode::IntegerOverflow)? {
        return revert_with(sdk, ERR_EPOCH_NOT_YET_COMMITTABLE, &(target, current));
    }
    let selection_epoch = target.saturating_sub(2);
    let top = selected_validators_at(sdk, selection_epoch)?;
    let mut eligible = Vec::new();
    for validator in top {
        let keys = read_consensus_keys(sdk, validator)?;
        if !keys.peer_pubkey.is_zero() && keys.activation_epoch <= selection_epoch {
            eligible.push(validator);
        }
    }
    if submitted.len() != eligible.len() {
        return revert_with(
            sdk,
            ERR_COMMITTEE_LENGTH_MISMATCH,
            &(U256::from(eligible.len()), U256::from(submitted.len())),
        );
    }

    let mut previous_peer = B256::ZERO;
    for validator in &submitted {
        let keys = read_consensus_keys(sdk, *validator)?;
        if keys.peer_pubkey.is_zero() || keys.activation_epoch > selection_epoch {
            return revert_with(sdk, ERR_COMMITTEE_MEMBER_KEYLESS, validator);
        }
        if !eligible.contains(validator) {
            return revert_with(sdk, ERR_COMMITTEE_MEMBER_NOT_IN_ACTIVE_SET, validator);
        }
        // Peer-key order gives every producer one canonical committee encoding.
        if keys.peer_pubkey <= previous_peer {
            return revert_with(sdk, ERR_COMMITTEE_NOT_STRICTLY_ASCENDING, validator);
        }
        previous_peer = keys.peer_pubkey;
    }

    let changed = committee_changed(sdk, target, &submitted)?;
    let stored = storage.epoch_committees_accessor().entry(target);
    for validator in &submitted {
        stored.push_checked(sdk, *validator)?;
    }
    storage
        .dkg_qual_accessor()
        .entry(target)
        .set_checked(sdk, changed)?;
    storage
        .last_committed_epoch_p1_accessor()
        .set_checked(sdk, target.checked_add(1).ok_or(ExitCode::IntegerOverflow)?)?;
    prune_committees(sdk, current)?;
    events::EpochCommitteeCommitted {
        epoch: target,
        committee: submitted,
    }
    .emit(sdk)
}

pub fn get_dkg_qual<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    let epoch = decode::<U64Command>(input)?.value;
    write_abi(
        sdk,
        &staking_storage()
            .dkg_qual_accessor()
            .entry(epoch)
            .get_checked(sdk)?,
    )
}

pub fn resolve_signer<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    let command = decode::<EpochSignerCommand>(input)?;
    let committee = staking_storage()
        .epoch_committees_accessor()
        .entry(command.epoch);
    let len = committee.len_checked(sdk)?;
    if len == 0 {
        return revert_with(sdk, ERR_EPOCH_COMMITTEE_NOT_COMMITTED, &command.epoch);
    }
    if command.signer_idx as u64 >= len {
        return revert_with(
            sdk,
            ERR_SIGNER_INDEX_OUT_OF_RANGE,
            &(command.epoch, command.signer_idx, U256::from(len)),
        );
    }
    write_abi(
        sdk,
        &committee.at(command.signer_idx as u64).get_checked(sdk)?,
    )
}

fn read_committee<SDK: SharedAPI>(sdk: &SDK, epoch: u64) -> Result<Vec<Address>, ExitCode> {
    let committee = staking_storage().epoch_committees_accessor().entry(epoch);
    let len = committee.len_checked(sdk)?;
    let mut result = Vec::with_capacity(len as usize);
    for index in 0..len {
        result.push(committee.at(index).get_checked(sdk)?);
    }
    Ok(result)
}

pub fn get_epoch_committee<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    write_abi(
        sdk,
        &read_committee(sdk, decode::<U64Command>(input)?.value)?,
    )
}

pub fn get_epoch_committee_length<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    let epoch = decode::<U64Command>(input)?.value;
    write_abi(
        sdk,
        &U256::from(
            staking_storage()
                .epoch_committees_accessor()
                .entry(epoch)
                .len_checked(sdk)?,
        ),
    )
}

pub fn get_epoch_committee_with_stakes<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    let epoch = decode::<U64Command>(input)?.value;
    let validators = read_committee(sdk, epoch)?;
    let mut keys = Vec::with_capacity(validators.len());
    let mut stakes = Vec::with_capacity(validators.len());
    for validator in &validators {
        keys.push(read_consensus_keys(sdk, *validator)?);
        stakes.push(validator_total_at(sdk, *validator, epoch)?);
    }
    write_returns(sdk, &(validators, keys, stakes))
}
