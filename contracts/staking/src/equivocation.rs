//! Permissionless equivocation proofs and permanent validator tombstoning.

use alloc::vec::Vec;

use fluentbase_sdk::{
    bytes::BytesMut, codec::SolidityABI, keccak256, Address, Bytes, ContextReader, ExitCode,
    SharedAPI, U256,
};

use crate::{
    consts::*,
    events,
    storage::{
        current_epoch, remove_active, staking_storage, STATUS_ACTIVE, STATUS_JAIL, STATUS_NOT_FOUND,
    },
    types::{DecodedEvidence, EquivocationCommand},
    util::{
        decode_args, ensure_initialized, ensure_mutable, ensure_non_payable, revert, revert_with,
        safe_transfer, set_selection_visible,
    },
};

const BLS_SIG_DST: &[u8] = b"BLS_SIG_BLS12381G1_XMD:SHA-256_SSWU_RO_POP_";

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
    let mut encoded = BytesMut::new();
    SolidityABI::<T>::encode_function_args(params, &mut encoded)
        .map_err(|_| ExitCode::MalformedBuiltinParams)?;
    let mut input = selector.to_be_bytes().to_vec();
    input.extend_from_slice(&encoded);
    let result = sdk.call(target, U256::ZERO, &input, None);
    if !result.status.is_ok() {
        sdk.write(result.data);
        return Err(result.status);
    }
    Ok(result.data)
}

fn call_decode<SDK, T, R>(
    sdk: &mut SDK,
    target: Address,
    selector: u32,
    params: &T,
) -> Result<R, ExitCode>
where
    SDK: SharedAPI,
    T: fluentbase_sdk::codec::FunctionArgs<fluentbase_sdk::byteorder::BE, 32, true, false>,
    R: fluentbase_sdk::codec::Encoder<fluentbase_sdk::byteorder::BE, 32, true, false>,
{
    let output = external_call(sdk, target, selector, params)?;
    SolidityABI::<R>::decode(&output, 0).map_err(|_| ExitCode::MalformedBuiltinParams)
}

fn namespace<SDK: SharedAPI>(sdk: &SDK, kind: u8) -> Vec<u8> {
    let mut result = b"FLUENT_DPOS_V1_".to_vec();
    result.extend_from_slice(&sdk.context().block_chain_id().to_be_bytes());
    result.extend_from_slice(match kind {
        0 => b"_NOTARIZE",
        1 => b"_NULLIFY",
        _ => b"_FINALIZE",
    });
    result
}

fn seize_self_stake<SDK: SharedAPI>(
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
    if len == 0 {
        return Ok(());
    }
    let seized = queue.at(len - 1).amount_accessor().get_checked(sdk)?;
    if seized.is_zero() {
        return Ok(());
    }
    queue.clear_checked(sdk)?;
    delegation.delegate_gap_accessor().set_checked(sdk, 0)?;

    let config = storage.config_accessor();
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
) -> Result<(), ExitCode> {
    let storage = staking_storage();
    let config = storage.config_accessor();
    let decoder = config.evidence_decoder_accessor().get_checked(sdk)?;
    if decoder.is_zero() {
        return revert(sdk, ERR_EVIDENCE_DECODER_NOT_CONFIGURED);
    }
    let evidence =
        call_decode::<_, _, DecodedEvidence>(sdk, decoder, decoder_selector, &(command.evidence,))?;
    let committee = storage.epoch_committees_accessor().entry(evidence.epoch);
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
    if storage
        .tombstoned_accessor()
        .entry(validator)
        .get_checked(sdk)?
    {
        return revert_with(sdk, ERR_ALREADY_SLASHED_FOR_EQUIVOCATION, &validator);
    }
    let stored_key = storage
        .consensus_keys_accessor()
        .entry(validator)
        .bls_pubkey_accessor()
        .load(sdk)?;
    if stored_key.len() != BLS_PUBKEY_LENGTH {
        return revert_with(sdk, ERR_CONSENSUS_KEYS_NOT_SET, &validator);
    }
    let verifier = config.bls_verifier_accessor().get_checked(sdk)?;
    if verifier.is_zero() {
        return revert(sdk, ERR_BLS_VERIFIER_NOT_CONFIGURED);
    }
    let supplied_key = call_decode::<_, _, Vec<u8>>(
        sdk,
        verifier,
        SIG_BLS_COMPRESS_G2_UNCHECKED,
        &(command.pk_uncompressed.clone(),),
    )?;
    if keccak256(&supplied_key) != keccak256(&stored_key) {
        return revert(sdk, ERR_EQUIVOCATION_KEY_MISMATCH);
    }
    let supplied_sig1 = call_decode::<_, _, Vec<u8>>(
        sdk,
        verifier,
        SIG_BLS_COMPRESS_G1_UNCHECKED,
        &(command.sig1_uncompressed.clone(),),
    )?;
    let supplied_sig2 = call_decode::<_, _, Vec<u8>>(
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
            BLS_SIG_DST.to_vec(),
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
            BLS_SIG_DST.to_vec(),
            command.sig2_uncompressed,
            command.pk_uncompressed,
        ),
    )?;
    if !valid1 || !valid2 {
        return revert(sdk, ERR_EQUIVOCATION_SIGNATURE_INVALID);
    }

    // A verified conflict is terminal: the validator cannot re-register keys
    // or return through the ordinary jail-release path.
    storage
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
    let reporter = sdk.context().contract_caller();
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

pub fn slash_notarize<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    slash_equivocation(
        sdk,
        decode_equivocation(input)?,
        SIG_DECODE_CONFLICTING_NOTARIZE,
    )
}

pub fn slash_finalize<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    slash_equivocation(
        sdk,
        decode_equivocation(input)?,
        SIG_DECODE_CONFLICTING_FINALIZE,
    )
}

pub fn slash_nullify_finalize<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    slash_equivocation(
        sdk,
        decode_equivocation(input)?,
        SIG_DECODE_NULLIFY_FINALIZE,
    )
}

fn decode_equivocation(input: &[u8]) -> Result<EquivocationCommand, ExitCode> {
    let (evidence, pk_uncompressed, sig1_uncompressed, sig2_uncompressed) =
        decode_args::<(Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>)>(input)?;
    Ok(EquivocationCommand {
        evidence,
        pk_uncompressed,
        sig1_uncompressed,
        sig2_uncompressed,
    })
}
