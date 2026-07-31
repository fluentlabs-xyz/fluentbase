//! Consensus-key registration and deterministic epoch committee commits.

use crate::{
    consts::*,
    events, math,
    staking::{
        remove_active, selected_validators, selected_validators_at, selection_candidates_at,
        set_selection_visible, top_k_by_stake_at, touch_snapshot_at_or_before,
        validator_has_minimum_self_stake_at, validator_total_at,
    },
    storage::{chain_config_storage, consensus_storage, staking_storage},
    types::{
        AddressCommand, ConsensusKeys, DecodedEvidence, EpochSignerCommand, EquivocationCommand,
        U64Command,
    },
    util::{
        current_epoch, decode, decode_args, ensure_initialized, ensure_mutable, ensure_non_payable,
        next_epoch, revert, revert_with, safe_transfer, write_abi,
    },
};
use alloc::vec::Vec;
use fluentbase_sdk::{
    byteorder::BE,
    bytes::BytesMut,
    codec::{Encoder, FunctionArgs, SolidityABI},
    keccak256, Address, Bytes, ContextReader, ExitCode, SharedAPI, B256, U256,
};

const BLS_POP_DST: &[u8] = b"BLS_POP_BLS12381G1_XMD:SHA-256_SSWU_RO_POP_";

fn encode_external_call<T>(selector: u32, params: &T) -> Result<Vec<u8>, ExitCode>
where
    T: FunctionArgs<BE, 32, true, false>,
{
    let mut encoded = BytesMut::new();
    SolidityABI::<T>::encode_function_args(params, &mut encoded)
        .map_err(|_| ExitCode::MalformedBuiltinParams)?;
    let mut input = selector.to_be_bytes().to_vec();
    input.extend_from_slice(&encoded);
    Ok(input)
}

/// Encode a Solidity function's return tuple without an outer tuple offset.
fn write_returns<SDK, T>(sdk: &mut SDK, value: &T) -> Result<(), ExitCode>
where
    SDK: SharedAPI,
    T: FunctionArgs<BE, 32, true, false>,
{
    let mut output = BytesMut::new();
    SolidityABI::<T>::encode_function_args(value, &mut output)
        .map_err(|_| ExitCode::MalformedBuiltinParams)?;
    sdk.write(output.freeze());
    Ok(())
}

fn external_call<SDK, T>(
    sdk: &mut SDK,
    target: Address,
    selector: u32,
    params: &T,
) -> Result<Bytes, ExitCode>
where
    SDK: SharedAPI,
    T: FunctionArgs<BE, 32, true, false>,
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
    for index in 0..3 {
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
    bls_pubkey: [B256; 3],
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

    let verifier = chain_config_storage()
        .bls_verifier_accessor()
        .get_checked(sdk)?;
    if verifier.is_zero() {
        return revert(sdk, ERR_BLS_VERIFIER_NOT_CONFIGURED);
    }
    let compressed_output = external_call(
        sdk,
        verifier,
        SIG_BLS_COMPRESS_G2_UNCHECKED,
        &(bls_pubkey_uncompressed.clone(),),
    )?;
    let compressed = SolidityABI::<Bytes>::decode(&compressed_output, 0)
        .map_err(|_| ExitCode::MalformedBuiltinParams)?;
    if compressed.len() != BLS_PUBKEY_LENGTH {
        return revert(sdk, ERR_INVALID_CONSENSUS_KEY_ENCODING);
    }
    let bls_pubkey_hash = keccak256(&compressed);
    if !consensus
        .bls_pubkey_owner_accessor()
        .entry(bls_pubkey_hash)
        .get_checked(sdk)?
        .is_zero()
    {
        return revert_with(sdk, ERR_BLS_PUBKEY_ALREADY_IN_USE, &bls_pubkey_hash);
    }
    let verify_output = external_call(
        sdk,
        verifier,
        SIG_BLS_VERIFY,
        &(
            fluent_namespace(sdk),
            compressed.clone(),
            Bytes::from_static(BLS_POP_DST),
            bls_pop_uncompressed,
            bls_pubkey_uncompressed,
        ),
    )?;
    let valid = SolidityABI::<bool>::decode(&verify_output, 0)
        .map_err(|_| ExitCode::MalformedBuiltinParams)?;
    if !valid {
        return revert_with(sdk, ERR_INVALID_PROOF_OF_POSSESSION, &validator);
    }

    Ok(VerifiedConsensusKeys {
        bls_pubkey: [
            B256::from_slice(&compressed[..32]),
            B256::from_slice(&compressed[32..64]),
            B256::from_slice(&compressed[64..]),
        ],
        bls_pubkey_hash,
        encoded_bls_pubkey: compressed,
        peer_pubkey,
    })
}

/// Stores already-verified keys atomically with validator creation.
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
    // Recheck after verifier calls so a reentrant peer-key claim cannot be
    // overwritten when the external call returns.
    if !consensus
        .peer_pubkey_owner_accessor()
        .entry(verified.peer_pubkey)
        .get_checked(sdk)?
        .is_zero()
    {
        return revert_with(sdk, ERR_PEER_PUBKEY_ALREADY_IN_USE, &verified.peer_pubkey);
    }
    // Recheck after verifier calls so a reentrant BLS-key claim cannot be
    // overwritten when the external call returns.
    if !consensus
        .bls_pubkey_owner_accessor()
        .entry(verified.bls_pubkey_hash)
        .get_checked(sdk)?
        .is_zero()
    {
        return revert_with(
            sdk,
            ERR_BLS_PUBKEY_ALREADY_IN_USE,
            &verified.bls_pubkey_hash,
        );
    }
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

/// Public handler `0xd41c52eb` (`getValidatorsWithKeys`).
///
/// Returns selected validators together with their active consensus keys.
pub fn get_validators_with_keys<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    write_validators_with_keys(sdk, selected_validators(sdk)?, None)
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
    write_validators_with_keys(sdk, validators, None)
}

/// Public handler `0x7cfba9f3` (`getValidatorsWithKeysAt`).
///
/// Returns the selected validators and consensus keys for an epoch.
pub fn get_validators_with_keys_at<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    let epoch = decode::<U64Command>(input)?.value;
    write_validators_with_keys(sdk, selected_validators_at(sdk, epoch)?, Some(epoch))
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

/// Public handler `0x8bd070e4` (`committeeSelectionEpoch`).
///
/// Returns the validator-selection epoch used by the next committee commit.
pub fn committee_selection_epoch<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    let target = consensus_storage()
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
    let incumbent = consensus_storage()
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

fn has_active_consensus_keys_at<SDK: SharedAPI>(
    sdk: &SDK,
    validator: Address,
    epoch: u64,
) -> Result<bool, ExitCode> {
    let keys = read_consensus_keys(sdk, validator)?;
    Ok(!keys.peer_pubkey.is_zero() && keys.activation_epoch <= epoch)
}

fn selected_committee_at<SDK: SharedAPI>(sdk: &SDK, epoch: u64) -> Result<Vec<Address>, ExitCode> {
    let candidates = selection_candidates_at(sdk, epoch)?;
    let mut eligible = Vec::with_capacity(candidates.len());
    for validator in candidates {
        if has_active_consensus_keys_at(sdk, validator, epoch)? {
            eligible.push(validator);
        }
    }
    let cap = chain_config_storage()
        .active_validators_length_accessor()
        .get_checked(sdk)? as usize;
    top_k_by_stake_at(sdk, eligible, epoch, cap)
}

fn prune_committees<SDK: SharedAPI>(sdk: &mut SDK, current: u64) -> Result<(), ExitCode> {
    let storage = consensus_storage();
    let retention = chain_config_storage()
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

/// Public handler `0x87401d8a` (`commitEpochCommittee`).
///
/// Commits the next epoch committee after validating its membership and ordering.
pub fn commit_epoch_committee<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    if sdk.context().contract_caller() != SYSTEM_CALLER {
        return revert(sdk, ERR_ONLY_SYSTEM_CALL);
    }
    let (submitted,) = decode_args::<(Vec<Address>,)>(input)?;
    let storage = consensus_storage();
    let current = current_epoch(sdk)?;
    let target = storage
        .last_committed_epoch_p1_accessor()
        .get_checked(sdk)?;
    if target > current.checked_add(2).ok_or(ExitCode::IntegerOverflow)? {
        return revert_with(sdk, ERR_EPOCH_NOT_YET_COMMITTABLE, &(target, current));
    }
    let selection_epoch = target.saturating_sub(2);
    let eligible = selected_committee_at(sdk, selection_epoch)?;
    if eligible.len() < MIN_COMMITTEE_LENGTH {
        return revert_with(
            sdk,
            ERR_COMMITTEE_TOO_SMALL,
            &(U256::from(eligible.len()), U256::from(MIN_COMMITTEE_LENGTH)),
        );
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
    let committee_epoch_p1 = target.checked_add(1).ok_or(ExitCode::IntegerOverflow)?;
    for validator in &submitted {
        stored.push_checked(sdk, *validator)?;
        let latest_committee = staking_storage()
            .validators_accessor()
            .entry(*validator)
            .last_committee_epoch_p1_accessor();
        if latest_committee.get_checked(sdk)? < committee_epoch_p1 {
            latest_committee.set_checked(sdk, committee_epoch_p1)?;
        }
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

/// Public handler `0xd7f1733d` (`resolveSigner`).
///
/// Resolves a committee signer index to its validator address and consensus key.
pub fn resolve_signer<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    let command = decode::<EpochSignerCommand>(input)?;
    let committee = consensus_storage()
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
    let committee = consensus_storage().epoch_committees_accessor().entry(epoch);
    let len = committee.len_checked(sdk)?;
    let mut result = Vec::with_capacity(len as usize);
    for index in 0..len {
        result.push(committee.at(index).get_checked(sdk)?);
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

/// Public handler `0xe3e6cedc` (`getEpochCommitteeLength`).
///
/// Returns the committed committee size for an epoch.
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
            consensus_storage()
                .epoch_committees_accessor()
                .entry(epoch)
                .len_checked(sdk)?,
        ),
    )
}

/// Public handler `0xa4d160c1` (`getEpochCommitteeWithStakes`).
///
/// Returns an epoch committee with consensus keys and historical stakes.
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
// Liveness slashing, jail transitions, and bounded readmission.

fn ensure_liveness<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    let caller = sdk.context().contract_caller();
    let expected = chain_config_storage()
        .liveness_slashing_accessor()
        .get_checked(sdk)?;
    if expected.is_zero() || caller != expected {
        return revert(sdk, ERR_ONLY_LIVENESS_SLASHING);
    }
    Ok(())
}

fn remove_jailed<SDK: SharedAPI>(sdk: &mut SDK, validator: Address) -> Result<(), ExitCode> {
    let jailed = consensus_storage().jailed_validators_accessor();
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
    if !validator_has_minimum_self_stake_at(sdk, validator, next_epoch(sdk)?)? {
        return revert(sdk, ERR_OWNER_SELF_STAKE_BELOW_MINIMUM);
    }
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

/// Public handler `0x73a3dda6` (`releaseValidatorFromJail`).
///
/// Releases an eligible validator from jail at its owner's request.
pub fn release_validator_from_jail<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    let validator = decode::<AddressCommand>(input)?.value;
    let storage = staking_storage();
    if consensus_storage()
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

/// Public handler `0x67d80300` (`readmitExpiredJails`).
///
/// Readmits validators whose jail periods expired by the requested epoch.
pub fn readmit_expired_jails<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_liveness(sdk)?;
    let _: U64Command = decode(input)?;
    let epoch = current_epoch(sdk)?;
    let storage = staking_storage();
    let consensus = consensus_storage();
    let jailed = consensus.jailed_validators_accessor();
    let len = jailed.len_checked(sdk)?;
    if len == 0 {
        consensus
            .jailed_scan_cursor_accessor()
            .set_checked(sdk, 0)?;
        return Ok(());
    }
    let mut index = consensus.jailed_scan_cursor_accessor().get_checked(sdk)? % len;
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
        if consensus
            .tombstoned_accessor()
            .entry(validator)
            .get_checked(sdk)?
        {
            remove_jailed(sdk, validator)?;
        } else if record.status_accessor().get_checked(sdk)? == STATUS_JAIL
            && epoch >= record.jailed_before_accessor().get_checked(sdk)?
        {
            if validator_has_minimum_self_stake_at(sdk, validator, next_epoch(sdk)?)? {
                readmit(sdk, validator)?;
            } else {
                index = (index + 1) % current_len;
            }
        } else {
            index = (index + 1) % current_len;
        }
        examined += 1;
    }
    let remaining = jailed.len_checked(sdk)?;
    consensus
        .jailed_scan_cursor_accessor()
        .set_checked(sdk, if remaining == 0 { 0 } else { index % remaining })
}

fn quorum(n: u64) -> u64 {
    if n == 0 {
        return 0;
    }
    n - (n - 1) / 3
}

/// Returns the fixed committee size, its currently active members, and whether
/// `validator` belongs to it for the liveness incident at `epoch`.
///
/// A committed committee is the canonical checkpoint. Before that checkpoint
/// exists (notably in genesis/unit-test flows), epoch-selected membership is
/// already historical: liveness removals only change visibility from E+1.
fn liveness_committee_state<SDK: SharedAPI>(
    sdk: &SDK,
    epoch: u64,
    validator: Address,
) -> Result<(u64, u64, bool), ExitCode> {
    let consensus = consensus_storage();
    let committed = consensus
        .last_committed_epoch_p1_accessor()
        .get_checked(sdk)?
        > epoch;
    let committee = if committed {
        read_committee(sdk, epoch)?
    } else {
        selected_validators_at(sdk, epoch)?
    };
    let storage = staking_storage();
    let mut active_members = 0u64;
    let mut contains_validator = false;
    for member in &committee {
        if *member == validator {
            contains_validator = true;
        }
        if storage
            .validators_accessor()
            .entry(*member)
            .status_accessor()
            .get_checked(sdk)?
            == STATUS_ACTIVE
        {
            active_members += 1;
        }
    }
    Ok((committee.len() as u64, active_members, contains_validator))
}

/// Public handler `0xc96be4cb` (`slash`).
///
/// Applies liveness slashing to a validator on an authorized system call.
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

    let threshold = chain_config_storage()
        .felony_threshold_accessor()
        .get_checked(sdk)?;
    if slashes >= threshold {
        if status == STATUS_JAIL {
            let jail_until = epoch
                .checked_add(
                    chain_config_storage()
                        .validator_jail_epoch_length_accessor()
                        .get_checked(sdk)? as u64,
                )
                .ok_or(ExitCode::IntegerOverflow)?;
            let current_deadline = record.jailed_before_accessor().get_checked(sdk)?;
            record
                .jailed_before_accessor()
                .set_checked(sdk, core::cmp::max(current_deadline, jail_until))?;
            events::ValidatorJailed { validator, epoch }.emit(sdk)?;
        } else {
            let (committee_len, active_members, is_committee_member) =
                liveness_committee_state(sdk, epoch, validator)?;
            let quorum_floor = quorum(committee_len);
            if status == STATUS_ACTIVE
                && is_committee_member
                && (active_members == 0 || active_members - 1 < quorum_floor)
            {
                events::LivenessJailSkippedHaltGuard {
                    validator,
                    epoch,
                    active_set_size: U256::from(active_members),
                    quorum_floor: U256::from(quorum_floor),
                }
                .emit(sdk)?;
            } else {
                let jail_until = epoch
                    .checked_add(
                        chain_config_storage()
                            .validator_jail_epoch_length_accessor()
                            .get_checked(sdk)? as u64,
                    )
                    .ok_or(ExitCode::IntegerOverflow)?;
                record.status_accessor().set_checked(sdk, STATUS_JAIL)?;
                record
                    .jailed_before_accessor()
                    .set_checked(sdk, jail_until)?;
                remove_active(sdk, validator)?;
                consensus_storage()
                    .jailed_validators_accessor()
                    .push_checked(sdk, validator)?;
                set_selection_visible(sdk, validator, false, epoch)?;
                events::ValidatorJailed { validator, epoch }.emit(sdk)?;
            }
        }
    }
    events::ValidatorSlashed {
        validator,
        slashes,
        epoch,
    }
    .emit(sdk)
}
// Equivocation proofs and permanent validator tombstoning.

const BLS_SIG_DST: &[u8] = b"BLS_SIG_BLS12381G1_XMD:SHA-256_SSWU_RO_POP_";
const REPORT_COMMITMENT_DOMAIN: &[u8] = b"FluentStakingEquivocationReportV1";

/// Computes the commitment consumed by an equivocation reveal.
///
/// This is equivalent to:
/// `keccak256(abi.encode(domainHash, chainId, staking, proofKind,
/// keccak256(evidence), beneficiary, salt))`.
pub(crate) fn report_commitment_hash(
    chain_id: u64,
    staking: Address,
    proof_kind: u8,
    evidence_hash: B256,
    beneficiary: Address,
    salt: B256,
) -> B256 {
    let mut encoded = Vec::with_capacity(32 * 7);
    encoded.extend_from_slice(keccak256(REPORT_COMMITMENT_DOMAIN).as_slice());
    encoded.extend_from_slice(&U256::from(chain_id).to_be_bytes::<{ U256::BYTES }>());
    encoded.extend_from_slice(staking.into_word().as_slice());
    encoded.extend_from_slice(&U256::from(proof_kind).to_be_bytes::<{ U256::BYTES }>());
    encoded.extend_from_slice(evidence_hash.as_slice());
    encoded.extend_from_slice(beneficiary.into_word().as_slice());
    encoded.extend_from_slice(salt.as_slice());
    keccak256(&encoded)
}

fn command_commitment<SDK: SharedAPI>(
    sdk: &SDK,
    command: &EquivocationCommand,
    proof_kind: u8,
) -> B256 {
    report_commitment_hash(
        sdk.context().block_chain_id(),
        sdk.context().contract_address(),
        proof_kind,
        keccak256(&command.evidence),
        command.beneficiary,
        command.salt,
    )
}

pub(crate) fn verify_report_commitment<SDK: SharedAPI>(
    sdk: &mut SDK,
    command: &EquivocationCommand,
    proof_kind: u8,
) -> Result<(), ExitCode> {
    let beneficiary = command.beneficiary;
    if beneficiary.is_zero() {
        return revert(sdk, ERR_ZERO_EQUIVOCATION_BENEFICIARY);
    }
    let expected = command_commitment(sdk, command, proof_kind);
    let entry = consensus_storage()
        .equivocation_commitments_accessor()
        .entry(beneficiary);
    let stored = entry.commitment_accessor().get_checked(sdk)?;
    if stored.is_zero() {
        return revert_with(sdk, ERR_NO_EQUIVOCATION_COMMITMENT, &beneficiary);
    }
    if stored != expected {
        return revert_with(
            sdk,
            ERR_EQUIVOCATION_COMMITMENT_MISMATCH,
            &(beneficiary, stored, expected),
        );
    }
    let committed_at = entry.committed_at_accessor().get_checked(sdk)?;
    let current_block = sdk.context().block_number();
    if current_block <= committed_at {
        return revert_with(
            sdk,
            ERR_EQUIVOCATION_COMMITMENT_NOT_MATURE,
            &(beneficiary, committed_at, current_block),
        );
    }
    Ok(())
}

pub(crate) fn consume_report_commitment<SDK: SharedAPI>(
    sdk: &mut SDK,
    beneficiary: Address,
) -> Result<(), ExitCode> {
    let entry = consensus_storage()
        .equivocation_commitments_accessor()
        .entry(beneficiary);
    entry.commitment_accessor().set_checked(sdk, B256::ZERO)?;
    entry.committed_at_accessor().set_checked(sdk, 0)
}

/// Public handler `0x32890bc0` (`commitEquivocationReport`).
///
/// Commits an equivocation-report hash for the caller.
pub fn commit_report<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    let (commitment,) = decode_args::<(B256,)>(input)?;
    if commitment.is_zero() {
        return revert(sdk, ERR_ZERO_EQUIVOCATION_COMMITMENT);
    }
    let beneficiary = sdk.context().contract_caller();
    if beneficiary.is_zero() {
        return revert(sdk, ERR_ZERO_EQUIVOCATION_BENEFICIARY);
    }
    let committed_at = sdk.context().block_number();
    let entry = consensus_storage()
        .equivocation_commitments_accessor()
        .entry(beneficiary);
    entry.commitment_accessor().set_checked(sdk, commitment)?;
    entry
        .committed_at_accessor()
        .set_checked(sdk, committed_at)?;
    events::EquivocationReportCommitted {
        beneficiary,
        commitment,
        block_number: committed_at,
    }
    .emit(sdk)
}

/// Public handler `0xc289d76e` (`computeEquivocationReportCommitment`).
///
/// Computes the commitment hash for an equivocation report.
pub fn compute_report_commitment<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let (beneficiary, proof_kind, evidence_hash, salt) =
        decode_args::<(Address, u8, B256, B256)>(input)?;
    if beneficiary.is_zero() {
        return revert(sdk, ERR_ZERO_EQUIVOCATION_BENEFICIARY);
    }
    if proof_kind >= EQUIVOCATION_PROOF_KIND_COUNT {
        return revert_with(sdk, ERR_INVALID_EQUIVOCATION_PROOF_KIND, &proof_kind);
    }
    let commitment = report_commitment_hash(
        sdk.context().block_chain_id(),
        sdk.context().contract_address(),
        proof_kind,
        evidence_hash,
        beneficiary,
        salt,
    );
    write_returns(sdk, &(commitment,))
}

/// Public handler `0xa3aae5dd` (`getEquivocationReportCommitment`).
///
/// Returns a beneficiary's pending equivocation-report commitment.
pub fn get_report_commitment<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    let (beneficiary,) = decode_args::<(Address,)>(input)?;
    let entry = consensus_storage()
        .equivocation_commitments_accessor()
        .entry(beneficiary);
    write_returns(
        sdk,
        &(
            entry.commitment_accessor().get_checked(sdk)?,
            entry.committed_at_accessor().get_checked(sdk)?,
        ),
    )
}

fn call_decode<SDK, T, R>(
    sdk: &mut SDK,
    target: Address,
    selector: u32,
    params: &T,
) -> Result<R, ExitCode>
where
    SDK: SharedAPI,
    T: FunctionArgs<BE, 32, true, false>,
    R: Encoder<BE, 32, true, false>,
{
    let output = external_call(sdk, target, selector, params)?;
    SolidityABI::<R>::decode(&output, 0).map_err(|_| ExitCode::MalformedBuiltinParams)
}

fn namespace<SDK: SharedAPI>(sdk: &SDK, kind: u8) -> Bytes {
    let mut result = b"FLUENT_DPOS_V1_".to_vec();
    result.extend_from_slice(&sdk.context().block_chain_id().to_be_bytes());
    result.extend_from_slice(match kind {
        0 => b"_NOTARIZE",
        1 => b"_NULLIFY",
        _ => b"_FINALIZE",
    });
    Bytes::from(result)
}

pub(crate) fn seize_self_stake<SDK: SharedAPI>(
    sdk: &mut SDK,
    validator: Address,
    owner: Address,
    reporter: Address,
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

    queue.clear_checked(sdk)?;
    delegation.delegate_gap_accessor().set_checked(sdk, 0)?;
    undelegates.clear_checked(sdk)?;
    delegation.undelegate_gap_accessor().set_checked(sdk, 0)?;
    pending_undelegated.set_checked(sdk, U256::ZERO)?;

    let config = chain_config_storage();
    let stored_bps = config
        .slash_reporter_reward_bps_accessor()
        .get_checked(sdk)?;
    let bps = if stored_bps == 0 {
        DEFAULT_SLASH_REPORTER_REWARD_BPS
    } else {
        stored_bps
    };
    let mut reporter_reward = seized
        .checked_mul(U256::from(bps))
        .ok_or(ExitCode::IntegerOverflow)?
        / U256::from(10_000);
    let mut remainder = seized - reporter_reward;
    if reporter.is_zero() {
        remainder = remainder
            .checked_add(reporter_reward)
            .ok_or(ExitCode::IntegerOverflow)?;
        reporter_reward = U256::ZERO;
    } else {
        safe_transfer(sdk, reporter, reporter_reward)?;
    }
    let configured_fund = config.slash_fund_address_accessor().get_checked(sdk)?;
    let recipient = if configured_fund.is_zero() {
        EQUIVOCATION_BURN_SINK
    } else {
        configured_fund
    };
    safe_transfer(sdk, recipient, remainder)?;
    events::EquivocationStakeSeized {
        validator,
        reporter,
        reporter_reward,
        remainder,
        recipient,
    }
    .emit(sdk)
}

fn slash_equivocation<SDK: SharedAPI>(
    sdk: &mut SDK,
    command: EquivocationCommand,
    decoder_selector: u32,
    proof_kind: u8,
) -> Result<(), ExitCode> {
    verify_report_commitment(sdk, &command, proof_kind)?;
    let storage = staking_storage();
    let consensus = consensus_storage();
    let config = chain_config_storage();
    let decoder = config.evidence_decoder_accessor().get_checked(sdk)?;
    if decoder.is_zero() {
        return revert(sdk, ERR_EVIDENCE_DECODER_NOT_CONFIGURED);
    }
    let evidence =
        call_decode::<_, _, DecodedEvidence>(sdk, decoder, decoder_selector, &(command.evidence,))?;
    let committee = consensus.epoch_committees_accessor().entry(evidence.epoch);
    let committee_len = committee.len_checked(sdk)?;
    if committee_len == 0 {
        return revert_with(sdk, ERR_EPOCH_COMMITTEE_NOT_COMMITTED, &evidence.epoch);
    }
    if evidence.signer_idx as u64 >= committee_len {
        return revert_with(
            sdk,
            ERR_SIGNER_INDEX_OUT_OF_RANGE,
            &(
                evidence.epoch,
                evidence.signer_idx,
                U256::from(committee_len),
            ),
        );
    }
    let validator = committee.at(evidence.signer_idx as u64).get_checked(sdk)?;
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
    let stored_key = registered_keys.bls_pubkey;
    let verifier = config.bls_verifier_accessor().get_checked(sdk)?;
    if verifier.is_zero() {
        return revert(sdk, ERR_BLS_VERIFIER_NOT_CONFIGURED);
    }
    let supplied_key = call_decode::<_, _, Bytes>(
        sdk,
        verifier,
        SIG_BLS_COMPRESS_G2_UNCHECKED,
        &(command.pk_uncompressed.clone(),),
    )?;
    if keccak256(&supplied_key) != keccak256(&stored_key) {
        return revert(sdk, ERR_EQUIVOCATION_KEY_MISMATCH);
    }
    let supplied_sig1 = call_decode::<_, _, Bytes>(
        sdk,
        verifier,
        SIG_BLS_COMPRESS_G1_UNCHECKED,
        &(command.sig1_uncompressed.clone(),),
    )?;
    let supplied_sig2 = call_decode::<_, _, Bytes>(
        sdk,
        verifier,
        SIG_BLS_COMPRESS_G1_UNCHECKED,
        &(command.sig2_uncompressed.clone(),),
    )?;
    if keccak256(&supplied_sig1) != keccak256(&evidence.sig1)
        || keccak256(&supplied_sig2) != keccak256(&evidence.sig2)
    {
        return revert(sdk, ERR_EQUIVOCATION_SIGNATURE_INVALID);
    }
    let valid1 = call_decode::<_, _, bool>(
        sdk,
        verifier,
        SIG_BLS_VERIFY,
        &(
            namespace(sdk, evidence.kind1),
            evidence.msg1,
            Bytes::from_static(BLS_SIG_DST),
            command.sig1_uncompressed,
            command.pk_uncompressed.clone(),
        ),
    )?;
    let valid2 = call_decode::<_, _, bool>(
        sdk,
        verifier,
        SIG_BLS_VERIFY,
        &(
            namespace(sdk, evidence.kind2),
            evidence.msg2,
            Bytes::from_static(BLS_SIG_DST),
            command.sig2_uncompressed,
            command.pk_uncompressed,
        ),
    )?;
    if !valid1 || !valid2 {
        return revert(sdk, ERR_EQUIVOCATION_SIGNATURE_INVALID);
    }

    // A verified conflict is terminal: the validator cannot re-register keys
    // or return through the ordinary jail-release path.
    consensus
        .tombstoned_accessor()
        .entry(validator)
        .set_checked(sdk, true)?;
    let record = storage.validators_accessor().entry(validator);
    let status = record.status_accessor().get_checked(sdk)?;
    if status == STATUS_NOT_FOUND {
        return revert_with(sdk, ERR_VALIDATOR_NOT_FOUND, &validator);
    }
    if status == STATUS_ACTIVE {
        remove_active(sdk, validator)?;
    }
    record.status_accessor().set_checked(sdk, STATUS_JAIL)?;
    let penalty_epoch = current_epoch(sdk)?;
    set_selection_visible(sdk, validator, false, penalty_epoch)?;
    let owner = record.owner_accessor().get_checked(sdk)?;
    let reporter = command.beneficiary;
    consume_report_commitment(sdk, reporter)?;
    seize_self_stake(sdk, validator, owner, reporter)?;
    events::ValidatorJailed {
        validator,
        epoch: penalty_epoch,
    }
    .emit(sdk)?;
    events::EquivocationSlashed {
        validator,
        epoch: evidence.epoch,
        reporter,
    }
    .emit(sdk)
}

/// Public handler `0x2bc5fb10` (`slashEquivocationNotarize`).
///
/// Verifies conflicting notarizations and applies equivocation slashing.
pub fn slash_notarize<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    slash_equivocation(
        sdk,
        decode_equivocation(input)?,
        SIG_DECODE_CONFLICTING_NOTARIZE,
        EQUIVOCATION_PROOF_KIND_NOTARIZE,
    )
}

/// Public handler `0xb034c58b` (`slashEquivocationFinalize`).
///
/// Verifies conflicting finalizations and applies equivocation slashing.
pub fn slash_finalize<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    slash_equivocation(
        sdk,
        decode_equivocation(input)?,
        SIG_DECODE_CONFLICTING_FINALIZE,
        EQUIVOCATION_PROOF_KIND_FINALIZE,
    )
}

/// Public handler `0x337e1437` (`slashEquivocationNullifyFinalize`).
///
/// Verifies a nullify/finalize conflict and applies equivocation slashing.
pub fn slash_nullify_finalize<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    slash_equivocation(
        sdk,
        decode_equivocation(input)?,
        SIG_DECODE_NULLIFY_FINALIZE,
        EQUIVOCATION_PROOF_KIND_NULLIFY_FINALIZE,
    )
}

pub(crate) fn decode_equivocation(input: &[u8]) -> Result<EquivocationCommand, ExitCode> {
    let (evidence, pk_uncompressed, sig1_uncompressed, sig2_uncompressed, beneficiary, salt) =
        decode_args::<(Bytes, Bytes, Bytes, Bytes, Address, B256)>(input)?;
    Ok(EquivocationCommand {
        evidence,
        pk_uncompressed,
        sig1_uncompressed,
        sig2_uncompressed,
        beneficiary,
        salt,
    })
}
