//! In-process read of the Fluent staking system contract from the node's own
//! reth state at an explicit block hash, decoded into hybrid types.
//!
//! Generic over reth traits rather than over `fluentbase-node`, so this crate
//! stays out of a dependency cycle.

use alloy_consensus::BlockHeader;
use alloy_evm::Evm;
use alloy_primitives::{Address, Bytes, B256, U256};
use alloy_sol_types::SolCall;
use commonware_codec::DecodeExt as _;
use fluentbase_bls::{BlsPubkey, PeerPubkey, PUBKEY_BYTES};
use reth_evm::{ConfigureEvm, EvmFor};
use reth_primitives_traits::HeaderTy;
use reth_revm::{
    database::StateProviderDatabase,
    revm::context::result::{ExecutionResult, Output},
};
use reth_storage_api::{
    errors::{db::DatabaseError, provider::ProviderError},
    AccountReader, HeaderProvider, StateProviderBox, StateProviderFactory,
};

use crate::error::{ReadError, SHORT_READ_DISPLAY, SNAPSHOT_DISPLAY, TORN_RANGE_DISPLAY};

/// Classify the error from a `state_by_block_hash` read: reth answers
/// `StateForHashNotFound` while the header exists but the executed state does not
/// (pipeline backfill, or an unwind), which consumers must see as a transient
/// deferral rather than an opaque `Backend` string. Every other provider error
/// keeps the `Backend` mapping.
fn map_state_provider_err(e: ProviderError) -> ReadError {
    classify_transient_provider_error(&e).unwrap_or_else(|| ReadError::Backend(e.to_string()))
}

/// Classify a reth [`ProviderError`] observed at a read boundary into the transient
/// taxonomy, or `None` for a genuine backend fault the caller maps to
/// [`ReadError::Backend`].
///
/// `pub` because the consensus anchor probe hits the same reth storage for an
/// executed hash, so it has to reach the same verdict from this table rather than
/// re-derive one.
///
/// Two transient shapes:
/// - a clean state-miss (`StateForHashNotFound`) → [`ReadError::StateNotMaterialized`];
/// - a torn static-file read (the persistence thread appending concurrently) →
///   [`ReadError::TransientStorage`], covering the typed `DatabaseError::Decode`
///   changeset row and the inconsistent-range / short-read / inconsistent-snapshot
///   reads that arrive as `ProviderError::Other(AnyError)` and are matched by the
///   display constants below.
pub fn classify_transient_provider_error(e: &ProviderError) -> Option<ReadError> {
    match e {
        ProviderError::StateForHashNotFound(hash) => {
            Some(ReadError::StateNotMaterialized { hash: *hash })
        }
        ProviderError::Database(DatabaseError::Decode) => {
            Some(ReadError::TransientStorage(e.to_string()))
        }
        ProviderError::Other(inner) => {
            let display = inner.to_string();
            (display.contains(TORN_RANGE_DISPLAY)
                || display.contains(SHORT_READ_DISPLAY)
                || display.contains(SNAPSHOT_DISPLAY))
            .then_some(ReadError::TransientStorage(display))
        }
        _ => None,
    }
}

/// Classify a `transact_system_call` failure. The EVM error's `source()` chain
/// preserves the typed reth `ProviderError`, so a transient storage read is
/// recovered typed through [`classify_transient_provider_error`] rather than as a
/// stringly-typed guess; anything else stays [`ReadError::Backend`], including a
/// fork that wraps the database error and never yields a `ProviderError`.
fn map_evm_call_err(e: &(dyn std::error::Error + 'static)) -> ReadError {
    let mut source = Some(e);
    while let Some(err) = source {
        if let Some(provider_err) = err.downcast_ref::<ProviderError>() {
            return classify_transient_provider_error(provider_err)
                .unwrap_or_else(|| ReadError::Backend(e.to_string()));
        }
        source = err.source();
    }
    ReadError::Backend(e.to_string())
}

/// The staking system contract's ABI, imported from the one declaration both sides
/// compile against (`fluentbase-staking-abi`): the rWasm dispatcher matches raw
/// 4-byte selectors, so a signature typo is an `ERR_UNKNOWN_METHOD` revert against
/// a live chain rather than a mis-decode, which is why this is not a second `sol!`
/// block.
///
/// Aliased as a module so the Solidity `ConsensusKeys` tuple does not collide with
/// the hybrid [`ConsensusKeys`] below (same identifier, different types).
use fluentbase_staking_abi as abi;
use fluentbase_types::staking_protocol;

/// Smallest committee the contract will commit: `setActiveValidatorsLength`
/// refuses a cap under it and `commitEpochCommittee` reverts `CommitteeTooSmall`
/// on a shorter selection, so a shorter non-empty committee cannot be a legal
/// on-chain state and the read-side check below is a hard error, not a warning.
pub use staking_protocol::MIN_COMMITTEE_LENGTH;

/// On-chain `totalDelegated` is wei-scale (the contract compacts it by `1e10`);
/// dividing by this recovers the compacted `uint112` the elector ranks on. A drift
/// mis-weights leaders on every node equally, so no fork would reveal it.
pub use staking_protocol::BALANCE_COMPACT_PRECISION;

/// Upper bound on a compacted stake weight: the contract stores `totalDelegated`
/// compacted in a `uint112`, so a value at or above `2^112` is not a legal on-chain
/// state and must be rejected rather than carried into the elector.
///
/// Derived from the same `COMPACT_STAKE_BITS` the contract's storage width is built
/// from, so the elector's overflow argument does not rest on two literals that
/// happen to agree.
use staking_protocol::MAX_COMPACT_STAKE;

/// Wei-scale `totalDelegated` → compacted `uint112` weight. Delegations are exact
/// multiples of [`BALANCE_COMPACT_PRECISION`], so the division is lossless.
fn compact_stake(wei: U256) -> Result<u128, ReadError> {
    let compacted = u128::try_from(wei / U256::from(BALANCE_COMPACT_PRECISION))
        .map_err(|_| ReadError::AbiDecode("stake exceeds u128".into()))?;
    if compacted >= MAX_COMPACT_STAKE {
        return Err(ReadError::AbiDecode(
            "compacted stake exceeds the contract's uint112 range".into(),
        ));
    }
    Ok(compacted)
}

/// A validator's consensus identity, decoded and validated: `bls_pubkey` is
/// subgroup-checked on decode, `peer_pubkey` is a 32-byte ed25519 key.
///
/// Order in any `Vec` is contract order verbatim — this crate never sorts — which
/// [`check_committee_ordering`] asserts for a committee snapshot on the raw bytes.
#[derive(Clone, Debug)]
pub struct ConsensusKeys {
    pub bls_pubkey: BlsPubkey,
    pub peer_pubkey: PeerPubkey,
    pub activation_epoch: u64,
}

#[derive(Clone, Debug)]
pub struct ValidatorWithKeys {
    pub address: Address,
    pub keys: ConsensusKeys,
    /// Whether this member has been slashed for equivocation.
    ///
    /// The one field of the snapshot not frozen at the epoch commit: it is read
    /// live at the snapshot's own block, so a verdict landing mid-epoch reaches the
    /// committee it names. Two nodes reading the same epoch at different heights can
    /// therefore disagree for the few blocks the verdict takes to reach them both;
    /// the value is monotone on chain, so the disagreement resolves one way only,
    /// and nothing needing byte-identical snapshots across nodes may depend on it.
    pub tombstoned: bool,
}

/// Validator set as read at one specific block; `epoch` is computed locally from
/// `block_number` (see [`epoch_at_block`]), never read back from the contract.
#[derive(Clone, Debug)]
pub struct ValidatorSetSnapshot {
    pub block_hash: B256,
    pub block_number: u64,
    pub epoch: u64,
    pub validators: Vec<ValidatorWithKeys>,
    /// Frozen leader weights, or `None` once the contract's weight ring has wrapped
    /// past this epoch.
    ///
    /// Absent is not a vector of zeros: the leader elector fails rather than falling
    /// back to a uniform lottery, which would split leaders per node silently. When
    /// present the length equals `validators`.
    pub weights: Option<Vec<u128>>,
}

/// Startup configuration: the one staking system-contract address, parsed from the
/// operator's JSON file. The registry, epoch committee, chain-config views and
/// liveness recorder are one rWasm contract, hence one address.
///
/// No serde default, deliberately: a defaulted address would land on a codeless
/// account, and an EVM call to a codeless account returns Success — a silent no-op.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct StakingReaderConfig {
    pub staking_address: Address,
}

impl StakingReaderConfig {
    pub fn from_json_path(path: &std::path::Path) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        serde_json::from_slice(&bytes)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}

/// Relative DPoS epoch, from the one definition the contract computes its own
/// epochs with: [`staking_protocol::epoch_at_block`].
///
/// Activation `0` is the one input the two sides read differently — the contract
/// as the unarmed sentinel, the in-memory test mocks as absolute numbering — and
/// [`RethStakingStateReader::scheduled_dpos_activation`] keeps a real node out of
/// it.
pub use staking_protocol::epoch_at_block;

/// Activation-relative epoch-boundary predicate: `true` when `block_number` is the
/// last block of its relative epoch. Argument order matches [`epoch_at_block`]
/// deliberately — both take three `u64`s, so a swap compiles.
///
/// A pre-activation block returns `false`, matching the consensus `OriginEpocher`;
/// the two epoch authorities must agree.
#[inline]
pub fn is_epoch_boundary(
    block_number: u64,
    dpos_activation_block: u64,
    epoch_block_interval: u64,
) -> bool {
    // `activation == 0` (absolute numbering, mocks) is unaffected: no block is `< 0`.
    if block_number < dpos_activation_block {
        return false;
    }
    (block_number + 1 - dpos_activation_block).is_multiple_of(epoch_block_interval)
}

/// Tracker-feed guard: the peer set fed to commonware's `Oracle::track` must fit
/// its `max_peer_set_size`, or `track` panics deep in the p2p actor; this turns
/// that into an actionable [`ReadError::PeerSetTooLarge`].
pub(crate) fn check_peer_set_size(
    epoch: u64,
    size: usize,
    max_peer_set_size: usize,
) -> Result<(), ReadError> {
    if size > max_peer_set_size {
        return Err(ReadError::PeerSetTooLarge {
            epoch,
            size,
            max: max_peer_set_size,
        });
    }
    Ok(())
}

/// Assert the committee's order on arrival: it is the consensus index space.
///
/// The node writes a commonware `Participant` index into the block and the contract
/// resolves the accused positionally from its own stored array. The two agree only
/// while both order by ascending raw peer-pubkey bytes; a drift slashes the wrong
/// validator in silence, and a [`ReadError`] is deferrable, so failing the read
/// costs a retry rather than a halt.
///
/// Two properties are load-bearing:
/// - it compares raw bytes (`as_ref()`), not `PeerPubkey: Ord`, because raw
///   byte-lex is the comparator the contract uses;
/// - an empty committee is not an error: callers branch on `validators.is_empty()`
///   as a routine deferral, so erroring here would turn the epoch-boundary park
///   into a degraded retry loop.
pub(crate) fn check_committee_ordering(
    epoch: u64,
    members: &[ValidatorWithKeys],
) -> Result<(), ReadError> {
    if members.is_empty() {
        return Ok(());
    }
    if members.len() < MIN_COMMITTEE_LENGTH {
        return Err(ReadError::CommitteeTooSmall {
            epoch,
            size: members.len(),
            min: MIN_COMMITTEE_LENGTH,
        });
    }
    for (i, pair) in members.windows(2).enumerate() {
        let (a, b): (&[u8], &[u8]) = (
            pair[0].keys.peer_pubkey.as_ref(),
            pair[1].keys.peer_pubkey.as_ref(),
        );
        match a.cmp(b) {
            std::cmp::Ordering::Less => {}
            std::cmp::Ordering::Equal => {
                return Err(ReadError::CommitteeDuplicatePeerKey {
                    epoch,
                    position: i + 1,
                    validator: pair[1].address,
                })
            }
            std::cmp::Ordering::Greater => {
                return Err(ReadError::CommitteeOutOfOrder {
                    epoch,
                    position: i + 1,
                    validator: pair[1].address,
                })
            }
        }
    }
    Ok(())
}

/// Decode one ABI `ConsensusKeys` tuple into the validated reader type. Keys go
/// through the subgroup-checked `fluentbase-bls` decoders, so a malformed 96-byte
/// blob is rejected here; an unset entry (`blsPubkey.len() == 0`) is not valid —
/// check [`is_unset`] first.
fn decode_consensus_keys(k: abi::ConsensusKeys) -> Result<ConsensusKeys, ReadError> {
    if k.blsPubkey.len() != PUBKEY_BYTES {
        return Err(ReadError::AbiDecode(format!(
            "blsPubkey length {} != {PUBKEY_BYTES}",
            k.blsPubkey.len()
        )));
    }
    let bls_pubkey =
        BlsPubkey::decode(k.blsPubkey.as_ref()).map_err(|e| ReadError::BlsKey(format!("{e:?}")))?;
    let peer_pubkey =
        PeerPubkey::decode(k.peerPubkey.as_slice()).map_err(|_| ReadError::PeerKey)?;
    Ok(ConsensusKeys {
        bls_pubkey,
        peer_pubkey,
        activation_epoch: k.activationEpoch,
    })
}

/// The contract's "validator has no consensus keys set" sentinel.
#[inline]
fn is_unset(k: &abi::ConsensusKeys) -> bool {
    k.blsPubkey.is_empty()
}

/// One read-only system call to `addr` with `calldata` against an already-built
/// EVM. View functions do not mutate and `transact_system_call` never commits, so
/// the returned state delta is discarded. The system-call path needs no caller
/// funding, nonce or gas, and the staking getters do not gate on `msg.sender`.
///
/// A free function over `&mut impl Evm` so it can run against either a one-shot
/// EVM or the EVM hoisted for a whole committee snapshot.
fn exec_view<Ev: Evm>(evm: &mut Ev, addr: Address, calldata: Bytes) -> Result<Bytes, ReadError> {
    let out = evm
        .transact_system_call(Address::ZERO, addr, calldata)
        .map_err(|e| map_evm_call_err(&e))?;

    match out.result {
        ExecutionResult::Success { output, .. } => match output {
            Output::Call(b) | Output::Create(b, _) => Ok(b),
        },
        ExecutionResult::Revert { output, .. } => Err(ReadError::CallReverted(
            alloy_primitives::hex::encode(output),
        )),
        ExecutionResult::Halt { reason, .. } => {
            Err(ReadError::CallReverted(format!("halt: {reason:?}")))
        }
    }
}

fn decode_view<Ev: Evm, C: SolCall>(
    evm: &mut Ev,
    addr: Address,
    call: &C,
) -> Result<C::Return, ReadError> {
    let ret = exec_view(evm, addr, call.abi_encode().into())?;
    C::abi_decode_returns(&ret).map_err(|e| ReadError::AbiDecode(e.to_string()))
}

/// In-process staking reader over a reth provider + EVM config.
///
/// Read parameters are not cached: they are governance-mutable on-chain, so
/// caching the first read forever would split consensus if governance changed one
/// while nodes are live. Re-reading costs one extra in-process system call.
#[derive(Clone, Debug)]
pub struct RethStakingStateReader<P, E> {
    provider: P,
    evm_config: E,
    cfg: StakingReaderConfig,
}

impl<P, E> RethStakingStateReader<P, E>
where
    P: StateProviderFactory + HeaderProvider<Header = HeaderTy<E::Primitives>> + Send + Sync,
    E: ConfigureEvm + Send + Sync,
{
    pub fn new(provider: P, evm_config: E, cfg: StakingReaderConfig) -> Self {
        Self {
            provider,
            evm_config,
            cfg,
        }
    }

    /// Build the EVM for the state at block `at` once and hand it to `f`: the header
    /// read, state-provider build and EVM construction are invariant at a fixed `at`,
    /// so a multi-call read reuses one.
    ///
    /// Scoped to one `at` per invocation, not a persistent cross-read cache, so the
    /// no-caching rule for governance-mutable parameters is unaffected.
    fn with_evm<R>(
        &self,
        at: B256,
        f: impl FnOnce(
            &mut EvmFor<E, StateProviderDatabase<StateProviderBox>>,
            &HeaderTy<E::Primitives>,
        ) -> Result<R, ReadError>,
    ) -> Result<R, ReadError> {
        let header = self
            .provider
            .header(at)
            .map_err(|e| ReadError::Backend(e.to_string()))?
            .ok_or(ReadError::BlockNotFound(at))?;
        let state = self
            .provider
            .state_by_block_hash(at)
            .map_err(map_state_provider_err)?;

        let db = StateProviderDatabase::new(state);
        let mut evm = self
            .evm_config
            .evm_for_block(db, &header)
            .map_err(|e| ReadError::Backend(e.to_string()))?;

        f(&mut evm, &header)
    }

    fn call<C: SolCall>(&self, addr: Address, call: &C, at: B256) -> Result<C::Return, ReadError> {
        self.with_evm(at, |evm, _header| decode_view(evm, addr, call))
    }

    pub fn epoch_block_interval(&self, at: B256) -> Result<u64, ReadError> {
        self.call(
            self.cfg.staking_address,
            &abi::getEpochBlockIntervalCall {},
            at,
        )
    }

    /// `getDposActivationBlock()` at block `at` — the origin for the relative DPoS
    /// epoch numbering. `setDposActivationBlock` requires `newValue >= block.number`,
    /// so `0` is the unscheduled sentinel and a live chain never stores it.
    pub fn dpos_activation_block(&self, at: B256) -> Result<u64, ReadError> {
        self.call(
            self.cfg.staking_address,
            &abi::getDposActivationBlockCall {},
            at,
        )
    }

    /// Activation height as a scheduling state: `Ok(None)` while the staking
    /// contract has no code at `at` (not installed yet) or while activation is
    /// unscheduled (`0`); `Ok(Some(h))` once governance has scheduled it.
    ///
    /// Without the code-presence probe, a codeless account would surface as an
    /// `AbiDecode` error on the empty return instead of deferring.
    pub fn scheduled_dpos_activation(&self, at: B256) -> Result<Option<u64>, ReadError> {
        let state = self
            .provider
            .state_by_block_hash(at)
            .map_err(map_state_provider_err)?;
        // reth normalizes no-code accounts to `None`; the KECCAK_EMPTY arm guards
        // unnormalized providers.
        let deployed = state
            .basic_account(&self.cfg.staking_address)
            .map_err(|e| ReadError::Backend(e.to_string()))?
            .is_some_and(|acc| {
                acc.bytecode_hash
                    .is_some_and(|h| h != alloy_consensus::constants::KECCAK_EMPTY)
            });
        if !deployed {
            return Ok(None);
        }
        Ok(match self.dpos_activation_block(at)? {
            0 => None,
            h => Some(h),
        })
    }

    /// `getActiveValidatorsLength()` at block `at` — the contract's configured
    /// active-set cap.
    pub fn active_validators_length(&self, at: B256) -> Result<u64, ReadError> {
        self.call(
            self.cfg.staking_address,
            &abi::getActiveValidatorsLengthCall {},
            at,
        )
    }

    /// `getDkgQual(epoch)` at block `at` — the on-chain committee-change bit for
    /// `epoch`, set deterministically by `commitEpochCommittee` at commit time, not
    /// by a permissionless marker tx. `true` means the committee changed at `epoch`
    /// (its DKG re-mints the beacon key); `false` means the prior key is carried
    /// forward. Immutable once the epoch's committee is committed.
    pub fn dkg_qual(&self, epoch: u64, at: B256) -> Result<bool, ReadError> {
        self.call(self.cfg.staking_address, &abi::getDkgQualCall { epoch }, at)
    }

    /// Snapshot of the frozen `epoch` committee — authoritative for the peer set,
    /// the slashing window and the leader weights — each member joined with its full
    /// consensus keys, at block `at`.
    ///
    /// One `getEpochCommitteeWithStakes` call returns the complete snapshot
    /// `(addrs, keys, stakes, tombstoned)`. Membership, keys and stakes are frozen at
    /// the epoch commit; `tombstoned` is read live at `at` (see
    /// [`ValidatorWithKeys::tombstoned`]). A keyless member is
    /// [`ReadError::CommitteeMemberKeyless`], never silently skipped; an empty or
    /// uncommitted epoch is a snapshot with no validators.
    ///
    /// `addrs`, `keys` and `tombstoned` must agree in length, or
    /// [`ReadError::AbiDecode`]. `stakes` is the one leg allowed to disagree, and
    /// only by being empty beside a non-empty `addrs` — the contract's way of saying
    /// the weight ring has wrapped past this epoch, decoded as `weights: None`. Any
    /// other length is an error.
    ///
    /// [`check_committee_ordering`] runs at this one site, so every consumer
    /// inherits the index-space invariant through the shared snapshot.
    pub fn epoch_committee_snapshot(
        &self,
        epoch: u64,
        at: B256,
    ) -> Result<ValidatorSetSnapshot, ReadError> {
        let staking = self.cfg.staking_address;
        let (block_number, validators, weights) = self.with_evm(at, |evm, header| {
            let block_number = header.number();
            let ret = decode_view(
                evm,
                staking,
                &abi::getEpochCommitteeWithStakesCall { epoch },
            )?;
            if ret.addrs.len() != ret.keys.len() || ret.addrs.len() != ret.tombstoned.len() {
                return Err(ReadError::AbiDecode(
                    "committee/keys/tombstoned length mismatch".into(),
                ));
            }
            let weights = match ret.stakes.len() {
                0 if !ret.addrs.is_empty() => None,
                n if n == ret.addrs.len() => Some(
                    ret.stakes
                        .into_iter()
                        .map(compact_stake)
                        .collect::<Result<Vec<_>, ReadError>>()?,
                ),
                _ => {
                    return Err(ReadError::AbiDecode(
                        "committee/stakes length mismatch".into(),
                    ))
                }
            };
            let validators = ret
                .addrs
                .into_iter()
                .zip(ret.keys)
                .zip(ret.tombstoned)
                .map(|((address, k), tombstoned)| {
                    if is_unset(&k) {
                        return Err(ReadError::CommitteeMemberKeyless {
                            epoch,
                            validator: address,
                        });
                    }
                    Ok(ValidatorWithKeys {
                        address,
                        keys: decode_consensus_keys(k)?,
                        tombstoned,
                    })
                })
                .collect::<Result<Vec<_>, ReadError>>()?;
            check_committee_ordering(epoch, &validators)?;
            Ok((block_number, validators, weights))
        })?;
        Ok(ValidatorSetSnapshot {
            block_hash: at,
            block_number,
            epoch,
            validators,
            weights,
        })
    }

    /// Peer keys of the full Active-status validator registry
    /// (`getRegistryWithKeys`), not the stake-weighted top-k committee, at block
    /// `at`. Feeds the consensus p2p tier-2 peer set, so every activated validator
    /// keeps consensus-plane connectivity. Keyless entries are skipped: unlike a
    /// committee member, a keyless registry entry is a legal transient state.
    pub fn active_registry_peers(&self, at: B256) -> Result<Vec<PeerPubkey>, ReadError> {
        let decoded = self.call(
            self.cfg.staking_address,
            &abi::getRegistryWithKeysCall {},
            at,
        )?;
        decoded
            .keys
            .into_iter()
            .filter(|k| !is_unset(k))
            .map(|k| PeerPubkey::decode(k.peerPubkey.as_slice()).map_err(|_| ReadError::PeerKey))
            .collect()
    }
}

/// Trait-ified read surface over [`RethStakingStateReader`]: the subset of staking
/// reads the consensus layer consumes, kept as a trait so those consumers stay
/// generic over the reader and can inject deterministic mocks in tests.
pub trait StakingStateRead {
    fn epoch_committee_snapshot(
        &self,
        epoch: u64,
        at: B256,
    ) -> Result<ValidatorSetSnapshot, ReadError>;

    /// `getEpochBlockInterval()` (blocks per epoch) at `at`.
    fn epoch_block_interval(&self, at: B256) -> Result<u64, ReadError>;

    /// `getDposActivationBlock()` (the relative-epoch origin) at `at`.
    fn dpos_activation_block(&self, at: B256) -> Result<u64, ReadError>;

    /// DPoS activation as a scheduling state: `None` means "not yet a DPoS chain
    /// at `at`", so freezing the geometry there would read a reverting or empty
    /// chain config.
    ///
    /// The default folds nothing, so the in-memory test mocks — which have a live
    /// synthetic chain config and use activation `0` for absolute numbering — stay
    /// scheduled; [`RethStakingStateReader`] overrides it with the code-presence
    /// probe and the `0`-sentinel fold.
    fn scheduled_dpos_activation(&self, at: B256) -> Result<Option<u64>, ReadError> {
        Ok(Some(self.dpos_activation_block(at)?))
    }

    /// Peer keys of the full Active validator registry (the tier-2 peer set),
    /// keyless-filtered.
    fn active_registry_peers(&self, at: B256) -> Result<Vec<PeerPubkey>, ReadError>;
}

impl<P, E> StakingStateRead for RethStakingStateReader<P, E>
where
    P: StateProviderFactory + HeaderProvider<Header = HeaderTy<E::Primitives>> + Send + Sync,
    E: ConfigureEvm + Send + Sync,
{
    fn epoch_committee_snapshot(
        &self,
        epoch: u64,
        at: B256,
    ) -> Result<ValidatorSetSnapshot, ReadError> {
        RethStakingStateReader::epoch_committee_snapshot(self, epoch, at)
    }
    fn epoch_block_interval(&self, at: B256) -> Result<u64, ReadError> {
        RethStakingStateReader::epoch_block_interval(self, at)
    }
    fn dpos_activation_block(&self, at: B256) -> Result<u64, ReadError> {
        RethStakingStateReader::dpos_activation_block(self, at)
    }
    /// The code-presence probe replaces the default: a codeless chain config or an
    /// unscheduled `0` activation maps to `None`, so the beacon-plane cold start
    /// defers instead of erroring.
    fn scheduled_dpos_activation(&self, at: B256) -> Result<Option<u64>, ReadError> {
        RethStakingStateReader::scheduled_dpos_activation(self, at)
    }
    fn active_registry_peers(&self, at: B256) -> Result<Vec<PeerPubkey>, ReadError> {
        RethStakingStateReader::active_registry_peers(self, at)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        abi, check_committee_ordering, check_peer_set_size, decode_consensus_keys, epoch_at_block,
        is_epoch_boundary, is_unset, map_evm_call_err, map_state_provider_err, staking_protocol,
        StakingReaderConfig, ValidatorWithKeys, MIN_COMMITTEE_LENGTH,
    };
    use crate::error::{ReadError, SHORT_READ_DISPLAY, TORN_RANGE_DISPLAY};
    use alloy_primitives::{address, hex, Address, Bytes, FixedBytes, B256, U256};
    use alloy_sol_types::SolCall;
    use commonware_codec::Encode as _;
    use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer};
    use commonware_math::algebra::Random as _;
    use fluentbase_bls::PeerPubkey;
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng;
    use reth_storage_api::errors::{db::DatabaseError, provider::ProviderError};

    /// The one input this side and the contract read differently: `activation == 0`
    /// is absolute numbering here — what the in-memory mocks mean by it — and the
    /// unarmed sentinel on chain, where it answers epoch 0 at any height. The gate
    /// in `scheduled_dpos_activation` keeps a real node out of this input.
    #[test]
    fn a_zero_activation_is_absolute_numbering_on_this_side() {
        assert_eq!(epoch_at_block(0, 0, 100), Some(0));
        assert_eq!(epoch_at_block(100, 0, 100), Some(1));
        assert_eq!(epoch_at_block(200, 0, 100), Some(2));
        assert_eq!(epoch_at_block(200, 0, 0), None);
    }

    #[test]
    fn pre_activation_is_never_a_boundary() {
        for n in 0..64u64 {
            assert!(
                !is_epoch_boundary(n, 64, 32),
                "pre-activation block {n} must not be a boundary"
            );
        }
    }

    #[test]
    fn real_boundaries_are_at_activation_plus_k_interval_minus_one() {
        for k in 1..5u64 {
            let last = 64 + k * 32 - 1;
            assert!(
                is_epoch_boundary(last, 64, 32),
                "block {last} is a boundary"
            );
            assert!(!is_epoch_boundary(last - 1, 64, 32));
            assert!(!is_epoch_boundary(last + 1, 64, 32));
        }
    }

    #[test]
    fn absolute_numbering_activation_zero_is_unchanged() {
        assert!(is_epoch_boundary(99, 0, 100));
        assert!(is_epoch_boundary(199, 0, 100));
        assert!(!is_epoch_boundary(0, 0, 100));
        assert!(!is_epoch_boundary(98, 0, 100));
        assert!(!is_epoch_boundary(100, 0, 100));
    }

    fn keys(seed: u64) -> abi::ConsensusKeys {
        let mut rng = StdRng::seed_from_u64(seed);
        let peer = Ed25519PrivateKey::random(&mut rng).public_key();
        let bls = fluentbase_bls::keys::ValidatorBlsKeypair::generate(&mut rng);
        abi::ConsensusKeys {
            blsPubkey: Bytes::copy_from_slice(&bls.public_bytes()),
            peerPubkey: FixedBytes::<32>::from_slice(peer.encode().as_ref()),
            activationEpoch: 7,
        }
    }

    #[test]
    fn valid_consensus_keys_decode() {
        let decoded = decode_consensus_keys(keys(1)).expect("valid keys must decode");
        assert_eq!(decoded.activation_epoch, 7);
    }

    #[test]
    fn unset_entry_is_detected_and_rejected() {
        let unset = abi::ConsensusKeys {
            blsPubkey: Bytes::new(),
            peerPubkey: FixedBytes::<32>::ZERO,
            activationEpoch: 0,
        };
        assert!(is_unset(&unset));
        assert!(matches!(
            decode_consensus_keys(unset),
            Err(ReadError::AbiDecode(_))
        ));
    }

    #[test]
    fn malformed_96_byte_bls_blob_rejected_by_subgroup_check() {
        let bad = abi::ConsensusKeys {
            blsPubkey: Bytes::from(vec![0xFFu8; fluentbase_bls::PUBKEY_BYTES]),
            peerPubkey: keys(2).peerPubkey,
            activationEpoch: 1,
        };
        assert!(!is_unset(&bad));
        assert!(matches!(
            decode_consensus_keys(bad),
            Err(ReadError::BlsKey(_))
        ));
    }

    #[test]
    fn peer_set_size_at_max_is_ok() {
        assert!(check_peer_set_size(7, 51, 51).is_ok());
        assert!(check_peer_set_size(7, 0, 0).is_ok());
    }

    #[test]
    fn peer_set_size_over_max_errors() {
        assert!(matches!(
            check_peer_set_size(9, 52, 51),
            Err(ReadError::PeerSetTooLarge {
                epoch: 9,
                size: 52,
                max: 51
            })
        ));
    }

    #[test]
    fn empty_committee_decodes_to_empty_snapshot() {
        let data = abi::getEpochCommitteeWithStakesCall::abi_encode_returns(
            &abi::getEpochCommitteeWithStakesReturn {
                addrs: vec![],
                keys: vec![],
                stakes: vec![],
                tombstoned: vec![],
            },
        );
        let ret = abi::getEpochCommitteeWithStakesCall::abi_decode_returns(&data)
            .expect("empty committee must decode");
        assert!(ret.addrs.is_empty());
        assert!(ret.keys.is_empty());
        assert!(ret.stakes.is_empty());
        assert!(ret.tombstoned.is_empty());
    }

    /// The contract reports the tombstone positionally, so a decode that dropped or
    /// shifted the leg would name the wrong validator.
    #[test]
    fn the_tombstone_leg_decodes_positionally() {
        let data = abi::getEpochCommitteeWithStakesCall::abi_encode_returns(
            &abi::getEpochCommitteeWithStakesReturn {
                addrs: vec![Address::with_last_byte(1), Address::with_last_byte(2)],
                keys: vec![keys(11), keys(12)],
                stakes: vec![U256::from(1u64), U256::from(2u64)],
                tombstoned: vec![true, false],
            },
        );
        let ret = abi::getEpochCommitteeWithStakesCall::abi_decode_returns(&data)
            .expect("committee must decode");
        assert_eq!(ret.tombstoned, vec![true, false]);
    }

    /// `n` members whose raw peer-pubkey bytes ascend — the only shape the
    /// invariant accepts. Real ed25519 keys, because `PeerPubkey::decode`
    /// point-validates and arbitrary 32-byte patterns do not decode.
    fn ascending_members(n: usize) -> Vec<ValidatorWithKeys> {
        let mut peers: Vec<PeerPubkey> = (0..n as u64)
            .map(|s| Ed25519PrivateKey::random(&mut StdRng::seed_from_u64(1000 + s)).public_key())
            .collect();
        peers.sort_by(|a, b| {
            let (a, b): (&[u8], &[u8]) = (a.as_ref(), b.as_ref());
            a.cmp(b)
        });
        peers
            .into_iter()
            .enumerate()
            .map(|(i, peer_pubkey)| {
                let mut keys = decode_consensus_keys(keys(i as u64)).expect("fixture keys decode");
                keys.peer_pubkey = peer_pubkey;
                ValidatorWithKeys {
                    address: Address::with_last_byte(i as u8 + 1),
                    keys,
                    tombstoned: false,
                }
            })
            .collect()
    }

    #[test]
    fn ascending_committee_passes_the_index_space_invariant() {
        assert!(check_committee_ordering(7, &ascending_members(MIN_COMMITTEE_LENGTH)).is_ok());
        assert!(check_committee_ordering(7, &ascending_members(2 * MIN_COMMITTEE_LENGTH)).is_ok());
    }

    /// An uncommitted epoch reads back `[]` and consumers park on it, so the
    /// carve-out is exactly *empty*, not *short*.
    #[test]
    fn an_empty_committee_is_not_an_ordering_error() {
        assert!(check_committee_ordering(7, &[]).is_ok());
        assert!(matches!(
            check_committee_ordering(7, &ascending_members(1)),
            Err(ReadError::CommitteeTooSmall {
                epoch: 7,
                size: 1,
                min: MIN_COMMITTEE_LENGTH
            })
        ));
    }

    #[test]
    fn a_descending_pair_is_rejected_with_its_position() {
        let mut members = ascending_members(MIN_COMMITTEE_LENGTH);
        members.swap(2, 3);
        let moved = members[3].address;
        assert!(matches!(
            check_committee_ordering(9, &members),
            Err(ReadError::CommitteeOutOfOrder {
                epoch: 9,
                position: 3,
                validator,
            }) if validator == moved
        ));
    }

    #[test]
    fn a_duplicate_peer_key_is_rejected_with_its_position() {
        let mut members = ascending_members(MIN_COMMITTEE_LENGTH);
        members[2].keys.peer_pubkey = members[1].keys.peer_pubkey.clone();
        let duplicate = members[2].address;
        assert!(matches!(
            check_committee_ordering(9, &members),
            Err(ReadError::CommitteeDuplicatePeerKey {
                epoch: 9,
                position: 2,
                validator,
            }) if validator == duplicate
        ));
    }

    /// Closes the loop on the index-space agreement: `check_committee_ordering`
    /// compares raw bytes because that is the contract's comparator, while consumers
    /// index through commonware's `BiMap`, which orders by `PeerPubkey: Ord`. The
    /// positional slash is correct only while the two orders agree.
    #[test]
    fn peer_pubkey_ord_is_raw_byte_lex() {
        let peers: Vec<PeerPubkey> = (0..24u64)
            .map(|s| Ed25519PrivateKey::random(&mut StdRng::seed_from_u64(s)).public_key())
            .collect();
        for a in &peers {
            for b in &peers {
                let (a_raw, b_raw): (&[u8], &[u8]) = (a.as_ref(), b.as_ref());
                assert_eq!(
                    a.cmp(b),
                    a_raw.cmp(b_raw),
                    "PeerPubkey: Ord diverged from raw byte-lex — the contract's \
                     committee order and commonware's Participant index no longer agree"
                );
            }
        }
    }

    /// Three `blsPubkey` blobs of different lengths — 96 / 1 / 33 bytes: equal
    /// lengths would give a uniform stride and could not catch a wrong head
    /// stride in an array of dynamic elements.
    fn three_keys() -> Vec<abi::ConsensusKeys> {
        vec![
            abi::ConsensusKeys {
                blsPubkey: Bytes::from(vec![0xa1u8; 96]),
                peerPubkey: FixedBytes::<32>::repeat_byte(0x11),
                activationEpoch: 7,
            },
            abi::ConsensusKeys {
                blsPubkey: Bytes::from_static(&[0xb2u8]),
                peerPubkey: FixedBytes::<32>::repeat_byte(0x22),
                activationEpoch: 8,
            },
            abi::ConsensusKeys {
                blsPubkey: Bytes::from(vec![0xc3u8; 33]),
                peerPubkey: FixedBytes::<32>::repeat_byte(0x33),
                activationEpoch: 9,
            },
        ]
    }

    fn three_addrs() -> Vec<Address> {
        vec![
            Address::with_last_byte(1),
            Address::with_last_byte(2),
            Address::with_last_byte(3),
        ]
    }

    /// Byte-level conformance for the four-array return, asserted in both
    /// directions: a head-stride defect produces plausible incorrect data rather
    /// than an error, shifting the keys against the addresses so the node signs on
    /// behalf of the wrong validator.
    ///
    /// The vector was generated independently with `cast abi-encode`, not
    /// re-encoded from the declaration under test (0x…01/02/03 abbreviated,
    /// `0xa1`×96 / `0xb2` / `0xc3`×33, peer keys `0x11`×32 / `0x22`×32 / `0x33`×32):
    ///
    ///     cast abi-encode "f(address[],(bytes,bytes32,uint64)[],uint256[],bool[])" \
    ///       "[0x…01,0x…02,0x…03]" \
    ///       "[(0xa1…,0x11…,7),(0xb2,0x22…,8),(0xc3…,0x33…,9)]" \
    ///       "[1,22,333]" "[false,true,false]"
    #[test]
    fn epoch_committee_return_matches_the_contract_abi_encoding() {
        let fixture = abi::getEpochCommitteeWithStakesReturn {
            addrs: three_addrs(),
            keys: three_keys(),
            stakes: vec![U256::from(1u64), U256::from(22u64), U256::from(333u64)],
            tombstoned: vec![false, true, false],
        };
        let expected = hex!(
            "0000000000000000000000000000000000000000000000000000000000000080
             0000000000000000000000000000000000000000000000000000000000000100
             00000000000000000000000000000000000000000000000000000000000003c0
             0000000000000000000000000000000000000000000000000000000000000440
             0000000000000000000000000000000000000000000000000000000000000003
             0000000000000000000000000000000000000000000000000000000000000001
             0000000000000000000000000000000000000000000000000000000000000002
             0000000000000000000000000000000000000000000000000000000000000003
             0000000000000000000000000000000000000000000000000000000000000003
             0000000000000000000000000000000000000000000000000000000000000060
             0000000000000000000000000000000000000000000000000000000000000140
             00000000000000000000000000000000000000000000000000000000000001e0
             0000000000000000000000000000000000000000000000000000000000000060
             1111111111111111111111111111111111111111111111111111111111111111
             0000000000000000000000000000000000000000000000000000000000000007
             0000000000000000000000000000000000000000000000000000000000000060
             a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1
             a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1
             a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1
             0000000000000000000000000000000000000000000000000000000000000060
             2222222222222222222222222222222222222222222222222222222222222222
             0000000000000000000000000000000000000000000000000000000000000008
             0000000000000000000000000000000000000000000000000000000000000001
             b200000000000000000000000000000000000000000000000000000000000000
             0000000000000000000000000000000000000000000000000000000000000060
             3333333333333333333333333333333333333333333333333333333333333333
             0000000000000000000000000000000000000000000000000000000000000009
             0000000000000000000000000000000000000000000000000000000000000021
             c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3
             c300000000000000000000000000000000000000000000000000000000000000
             0000000000000000000000000000000000000000000000000000000000000003
             0000000000000000000000000000000000000000000000000000000000000001
             0000000000000000000000000000000000000000000000000000000000000016
             000000000000000000000000000000000000000000000000000000000000014d
             0000000000000000000000000000000000000000000000000000000000000003
             0000000000000000000000000000000000000000000000000000000000000000
             0000000000000000000000000000000000000000000000000000000000000001
             0000000000000000000000000000000000000000000000000000000000000000"
        );
        assert_eq!(
            abi::getEpochCommitteeWithStakesCall::abi_encode_returns(&fixture),
            expected
        );

        let decoded = abi::getEpochCommitteeWithStakesCall::abi_decode_returns(&expected)
            .expect("the pinned vector must decode");
        assert_eq!(decoded.addrs, fixture.addrs);
        assert_eq!(decoded.keys, fixture.keys);
        assert_eq!(decoded.stakes, fixture.stakes);
        assert_eq!(decoded.tombstoned, fixture.tombstoned);
    }

    /// Byte-level conformance for the two-array registry return. The middle
    /// entry is the contract's keyless sentinel (empty `blsPubkey`) — a legal
    /// transient state that `active_registry_peers` filters out — so the vector
    /// also pins the three lengths 96 / 0 / 33.
    ///
    ///     cast abi-encode "f(address[],(bytes,bytes32,uint64)[])" \
    ///       "[0x…01,0x…02,0x…03]" \
    ///       "[(0xa1…,0x11…,7),(0x,0x00…,0),(0xc3…,0x33…,9)]"
    #[test]
    fn registry_return_matches_the_contract_abi_encoding() {
        let mut keys = three_keys();
        keys[1] = abi::ConsensusKeys {
            blsPubkey: Bytes::new(),
            peerPubkey: FixedBytes::<32>::ZERO,
            activationEpoch: 0,
        };
        let fixture = abi::getRegistryWithKeysReturn {
            addrs: three_addrs(),
            keys,
        };
        let expected = hex!(
            "0000000000000000000000000000000000000000000000000000000000000040
             00000000000000000000000000000000000000000000000000000000000000c0
             0000000000000000000000000000000000000000000000000000000000000003
             0000000000000000000000000000000000000000000000000000000000000001
             0000000000000000000000000000000000000000000000000000000000000002
             0000000000000000000000000000000000000000000000000000000000000003
             0000000000000000000000000000000000000000000000000000000000000003
             0000000000000000000000000000000000000000000000000000000000000060
             0000000000000000000000000000000000000000000000000000000000000140
             00000000000000000000000000000000000000000000000000000000000001c0
             0000000000000000000000000000000000000000000000000000000000000060
             1111111111111111111111111111111111111111111111111111111111111111
             0000000000000000000000000000000000000000000000000000000000000007
             0000000000000000000000000000000000000000000000000000000000000060
             a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1
             a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1
             a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1
             0000000000000000000000000000000000000000000000000000000000000060
             0000000000000000000000000000000000000000000000000000000000000000
             0000000000000000000000000000000000000000000000000000000000000000
             0000000000000000000000000000000000000000000000000000000000000000
             0000000000000000000000000000000000000000000000000000000000000060
             3333333333333333333333333333333333333333333333333333333333333333
             0000000000000000000000000000000000000000000000000000000000000009
             0000000000000000000000000000000000000000000000000000000000000021
             c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3
             c300000000000000000000000000000000000000000000000000000000000000"
        );
        assert_eq!(
            abi::getRegistryWithKeysCall::abi_encode_returns(&fixture),
            expected
        );

        let decoded = abi::getRegistryWithKeysCall::abi_decode_returns(&expected)
            .expect("the pinned vector must decode");
        assert_eq!(decoded.addrs, fixture.addrs);
        assert_eq!(decoded.keys, fixture.keys);
        assert!(!is_unset(&decoded.keys[0]));
        assert!(is_unset(&decoded.keys[1]));
        assert!(!is_unset(&decoded.keys[2]));
    }

    /// Compares the contract's fault budget against the budget commonware applies
    /// to every Simplex quorum (`N3f1::max_faults`), over the whole legal committee
    /// range — not a copy of either formula written out again here.
    ///
    /// `n == 0` is excluded on purpose: commonware panics there and the shared
    /// function answers `0`, and no caller reaches it.
    #[test]
    fn the_shared_fault_budget_is_commonwares_budget() {
        use commonware_utils::{Faults as _, N3f1};
        for n in 1..=(staking_protocol::MAX_COMMITTEE_SIZE as usize) {
            assert_eq!(
                staking_protocol::fault_tolerance(n) as u32,
                N3f1::max_faults(n),
                "fault budget disagrees with commonware at n = {n}"
            );
        }
        assert_eq!(N3f1::max_faults(MIN_COMMITTEE_LENGTH), 1);
        assert_eq!(N3f1::max_faults(MIN_COMMITTEE_LENGTH - 1), 0);
    }

    #[test]
    fn config_is_one_address() {
        let json = r#"{"staking_address": "0x0000000000000000000000000000000000520011"}"#;
        let cfg: StakingReaderConfig = serde_json::from_str(json).expect("config must parse");
        assert_eq!(
            cfg.staking_address,
            address!("0x0000000000000000000000000000000000520011")
        );
    }

    #[test]
    fn config_without_the_address_refuses_to_parse() {
        assert!(serde_json::from_str::<StakingReaderConfig>(r#"{}"#).is_err());
    }

    #[test]
    fn state_not_found_maps_to_state_not_materialized() {
        let hash = B256::repeat_byte(0x7);
        match map_state_provider_err(ProviderError::StateForHashNotFound(hash)) {
            ReadError::StateNotMaterialized { hash: got } => assert_eq!(got, hash),
            other => panic!("expected StateNotMaterialized, got {other:?}"),
        }
    }

    #[test]
    fn other_provider_errors_keep_backend_mapping() {
        assert!(matches!(
            map_state_provider_err(ProviderError::StateForNumberNotFound(42)),
            ReadError::Backend(_)
        ));
    }

    #[test]
    fn torn_changeset_decode_maps_to_transient_storage() {
        // A torn changeset row is a mid-append read, not corruption.
        assert!(matches!(
            map_state_provider_err(ProviderError::Database(DatabaseError::Decode)),
            ReadError::TransientStorage(_)
        ));
    }

    #[test]
    fn torn_inconsistent_range_other_maps_to_transient_storage() {
        // A `NippyJar` inconsistent-range read arrives as `Other(AnyError)`, matched
        // by the shared display constant.
        let torn = ProviderError::other(std::io::Error::other(format!(
            "{TORN_RANGE_DISPLAY} 152247225..0, data size: 152247702"
        )));
        assert!(matches!(
            map_state_provider_err(torn),
            ReadError::TransientStorage(_)
        ));
    }

    #[test]
    fn unrelated_other_provider_error_stays_backend() {
        let other = ProviderError::other(std::io::Error::other("disk on fire"));
        assert!(matches!(
            map_state_provider_err(other),
            ReadError::Backend(_)
        ));
    }

    /// Mirrors `revm`'s `EVMError::Database` source chain, so this exercises the
    /// exact walk `map_evm_call_err` performs on a real EVM error.
    #[derive(Debug)]
    struct EvmDbError(ProviderError);
    impl std::fmt::Display for EvmDbError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "database error: {}", self.0)
        }
    }
    impl std::error::Error for EvmDbError {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }

    #[test]
    fn evm_call_err_recovers_typed_transient_provider_error_from_source_chain() {
        let torn = ProviderError::other(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            SHORT_READ_DISPLAY,
        ));
        assert!(matches!(
            map_evm_call_err(&EvmDbError(torn)),
            ReadError::TransientStorage(_)
        ));
    }

    #[test]
    fn evm_call_err_without_provider_error_stays_backend() {
        let plain = std::io::Error::other("evm misconfig");
        assert!(matches!(map_evm_call_err(&plain), ReadError::Backend(_)));
    }
}
