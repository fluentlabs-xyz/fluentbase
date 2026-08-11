//! Consensus-key registration and deterministic epoch committee commits.

use crate::{
    consts::*,
    events, math,
    staking::{
        remove_active, selected_validators, selected_validators_at, set_selection_visible,
        validator_total_at,
    },
    storage::{chain_config_storage, consensus_storage, staking_storage},
    types::{
        AddressCommand, ConsensusKeys, DecodedEvidence, EpochSignerCommand, EquivocationCommand,
        U64Command,
    },
    util::{
        current_epoch, decode, decode_args, ensure_initialized, ensure_mutable, ensure_non_payable,
        revert, revert_with, safe_transfer, write_abi,
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

/// Committee for `epoch`: the selection view, retained to members whose
/// consensus keys are active by `epoch`.
///
/// The key filter runs *after* the stake cut, never before. The off-chain
/// deriver builds its array from `getValidatorsWithKeysAt`, which is this same
/// selection view with inactive keys zeroed, and drops the keyless entries
/// itself. Filtering before the cut would promote a lower-staked keyed
/// validator into the committee and disagree with the array the deriver
/// submits, so `commitEpochCommittee` would reject every honest submission.
fn selected_committee_at<SDK: SharedAPI>(sdk: &SDK, epoch: u64) -> Result<Vec<Address>, ExitCode> {
    let selected = selected_validators_at(sdk, epoch)?;
    let mut eligible = Vec::with_capacity(selected.len());
    for validator in selected {
        if has_active_consensus_keys_at(sdk, validator, epoch)? {
            eligible.push(validator);
        }
    }
    Ok(eligible)
}

fn prune_committees<SDK: SharedAPI>(sdk: &mut SDK, current: u64) -> Result<(), ExitCode> {
    let storage = consensus_storage();
    let mut cursor = storage.pruned_up_to_p1_accessor().get_checked(sdk)?;
    let mut deleted = 0;
    // Bound cleanup so a long-idle chain cannot make one system call unbounded.
    while deleted < 16 {
        let liability_end = storage
            .committee_liability_end_epochs_accessor()
            .entry(cursor);
        let liability_end_epoch = liability_end.get_checked(sdk)?;
        // Committee commits are sequential. A missing deadline therefore means
        // that the pruning cursor has reached the first uncommitted epoch.
        if liability_end_epoch == 0 || current < liability_end_epoch {
            break;
        }
        storage
            .epoch_committees_accessor()
            .entry(cursor)
            .clear_checked(sdk)?;
        // Both arrays are positional with each other, so pruning one without the
        // other manufactures the length mismatch the reader reverts on.
        storage
            .leader_stakes_accessor()
            .entry(cursor)
            .clear_checked(sdk)?;
        liability_end.set_checked(sdk, 0)?;
        cursor = cursor.checked_add(1).ok_or(ExitCode::IntegerOverflow)?;
        deleted += 1;
    }
    storage.pruned_up_to_p1_accessor().set_checked(sdk, cursor)
}

pub(crate) fn ensure_equivocation_evidence_unexpired<SDK: SharedAPI>(
    sdk: &mut SDK,
    epoch: u64,
) -> Result<(), ExitCode> {
    let liability_end_epoch = consensus_storage()
        .committee_liability_end_epochs_accessor()
        .entry(epoch)
        .get_checked(sdk)?;
    if liability_end_epoch == 0 {
        return revert_with(sdk, ERR_EPOCH_COMMITTEE_NOT_COMMITTED, &epoch);
    }
    if current_epoch(sdk)? >= liability_end_epoch {
        return revert_with(
            sdk,
            ERR_EQUIVOCATION_EVIDENCE_EXPIRED,
            &(epoch, liability_end_epoch),
        );
    }
    Ok(())
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
    if target
        > current
            .checked_add(MAX_COMMITTEE_LOOKAHEAD_EPOCHS)
            .ok_or(ExitCode::IntegerOverflow)?
    {
        return revert_with(sdk, ERR_EPOCH_NOT_YET_COMMITTABLE, &(target, current));
    }
    let selection_epoch = target.saturating_sub(MAX_COMMITTEE_LOOKAHEAD_EPOCHS);
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

    let liability_end_epoch = target
        .checked_add(
            chain_config_storage()
                .undelegate_period_accessor()
                .get_checked(sdk)?,
        )
        .and_then(|epoch| epoch.checked_add(EPOCH_COMMITTEE_RETENTION_MARGIN))
        .and_then(|epoch| epoch.checked_add(1))
        .ok_or(ExitCode::IntegerOverflow)?;
    let changed = committee_changed(sdk, target, &submitted)?;
    let stored = storage.epoch_committees_accessor().entry(target);
    let stored_stakes = storage.leader_stakes_accessor().entry(target);
    for validator in &submitted {
        stored.push_checked(sdk, *validator)?;
        // Weights are stamped from the SELECTION epoch, the same vintage that
        // ranked membership, so leader weight and committee order cannot
        // disagree about a member.
        let weight = math::compact_balance(validator_total_at(sdk, *validator, selection_epoch)?)
            .ok_or(ExitCode::IntegerOverflow)?;
        stored_stakes.push_checked(sdk, weight)?;
    }
    storage
        .committee_liability_end_epochs_accessor()
        .entry(target)
        .set_checked(sdk, liability_end_epoch)?;
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
    let frozen = consensus_storage().leader_stakes_accessor().entry(epoch);
    let frozen_len = frozen.len_checked(sdk)?;
    if frozen_len != validators.len() as u64 {
        // Deliberately no fallback to a live walk: that would restore the
        // height-dependent read this freeze exists to remove.
        return revert_with(
            sdk,
            ERR_LEADER_STAKES_LENGTH_MISMATCH,
            &(
                epoch,
                U256::from(validators.len()),
                U256::from(frozen_len),
            ),
        );
    }
    let mut keys = Vec::with_capacity(validators.len());
    let mut stakes = Vec::with_capacity(validators.len());
    for (index, validator) in validators.iter().enumerate() {
        keys.push(read_consensus_keys(sdk, *validator)?);
        stakes.push(math::expand_balance(
            frozen.at(index as u64).get_checked(sdk)?,
        ));
    }
    write_returns(sdk, &(validators, keys, stakes))
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
    storage
        .validators_accessor()
        .entry(validator)
        .self_stake_unlock_epoch_accessor()
        .set_checked(sdk, 0)?;
    if seized.is_zero() {
        return Ok(());
    }

    queue.clear_checked(sdk)?;
    delegation
        .claimed_through_epoch_accessor()
        .set_checked(sdk, 0)?;
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
    ensure_equivocation_evidence_unexpired(sdk, evidence.epoch)?;
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

    // A verified conflict is terminal: the validator cannot re-register keys.
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
