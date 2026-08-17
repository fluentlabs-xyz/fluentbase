//! Read layer: in-process read of the Fluent staking system contract from
//! the node's own reth state at an explicit block hash, decoded into hybrid
//! types.
//!
//! This is exactly the composition reth's own `eth_call` performs (`state at
//! block` → `StateProviderDatabase` → `ConfigureEvm` → `transact` → decode);
//! fluentbase already builds and serves that RPC, so this is standard
//! plumbing. Generic over reth traits — **not** over `fluentbase-node` — so
//! this crate stays out of a dependency cycle.

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

/// Classify the error from a `state_by_block_hash` read at the boundary, so
/// consumers see the TYPE of a not-yet-materialized state (defer + retry) rather
/// than an opaque `Backend` string. reth returns `StateForHashNotFound` when the
/// header exists but the executed state is absent — a transient during pipeline
/// backfill (headers run ahead of state; an unwind removes state even at/below
/// the finalized hash). Every other provider error keeps the `Backend` mapping.
fn map_state_provider_err(e: ProviderError) -> ReadError {
    classify_transient_provider_error(&e).unwrap_or_else(|| ReadError::Backend(e.to_string()))
}

/// Classify a reth [`ProviderError`] observed at a read boundary into the transient
/// taxonomy, or `None` for a genuine backend fault the caller maps to
/// [`ReadError::Backend`]. Two transient shapes, both of which return NO committee so
/// a consumer defers + retries rather than failing closed:
/// - a clean state-miss (`StateForHashNotFound`) → [`ReadError::StateNotMaterialized`];
/// - a torn STATIC-FILE read (persistence thread appending concurrently) →
///   [`ReadError::TransientStorage`]. The four manifestations mirror `crates/node`'s
///   `is_transient_torn_static_file_read`: a torn changeset row is the typed
///   `DatabaseError::Decode`; the inconsistent-range / short-read / inconsistent-
///   snapshot reads arrive as `ProviderError::Other(AnyError)` whose payload is
///   matched by the shared display constants (family-5: the string match lives HERE,
///   in the error-owning layer, never in consensus).
fn classify_transient_provider_error(e: &ProviderError) -> Option<ReadError> {
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
/// PRESERVES the typed reth `ProviderError` (`EVMError::Database(ProviderError)` →
/// `core::error::Error::source`), so a transient reth storage read (state-miss / torn
/// static-file read) is recovered TYPED via [`classify_transient_provider_error`] —
/// the SAME variants `map_state_provider_err` produces — never a stringly-typed guess.
/// Anything else (a real revert is handled by the caller on `Ok`; a genuine backend
/// fault) stays [`ReadError::Backend`]. Degrades safely: a fork that wraps the DB error
/// differently simply never yields a `ProviderError` and falls through to `Backend`.
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

/// Solidity-ABI subset this layer calls. All seven views live on the ONE staking
/// system contract — the former `ChainConfig` split is gone, so
/// [`StakingReaderConfig`] carries a single address.
///
/// Verified against the rWasm staking contract (`contracts/staking/src`, FLU-989
/// worktree; artefact provenance in
/// `devnet/local-dpos-smoke/contracts/STAKING_ARTEFACT.md`): `types.rs:111-115`
/// `ConsensusKeys`, `consensus.rs:309,584,683` and `config.rs:245,317,330,343` the
/// view handlers. The contract dispatches on a raw 4-byte selector, so a signature
/// typo here reverts `ERR_UNKNOWN_METHOD` rather than mis-decoding — the selectors
/// are pinned byte-for-byte in `tests::view_selectors_are_pinned`.
///
/// Kept as an inner module so the Solidity `ConsensusKeys` tuple does not
/// collide with the hybrid [`ConsensusKeys`] below (same identifier,
/// different types).
mod abi {
    use alloy_sol_types::sol;

    sol! {
        /// Mirrors the contract's `ConsensusKeys`. `blsPubkey` is exactly 96 B
        /// when set (compressed BLS12-381 G2, MinSig); empty when unset.
        #[derive(Debug, PartialEq)]
        struct ConsensusKeys {
            bytes blsPubkey;
            bytes32 peerPubkey;
            uint64 activationEpoch;
        }

        // Staking contract. `tombstoned` is the per-member equivocation
        // tombstone, read LIVE at `at` (the other three legs are frozen at the
        // commit): it rides this snapshot rather than a view of its own so the
        // flag arrives on the path the node already takes, at no extra call.
        //
        // KNOWN CONTRACT DRIFT — the fourth array has no contract-side
        // counterpart. `feat/flu-989-port-solidity-delta` (and
        // `origin/feat/flu-989-rust-staking`) end this handler with
        // `write_returns(sdk, &(validators, keys, stakes))` — THREE arrays.
        // Return types do not enter a selector, so `view_selectors_are_pinned`
        // below passes and proves nothing about this; the arity is pinned
        // separately by `epoch_committee_return_arity_is_pinned`. Do not
        // one-sidedly drop the leg — see the merge checklist at the top of
        // `crates/dpos/consensus/src/slasher/actor.rs`.
        function getEpochCommitteeWithStakes(uint64 epoch)
            external view returns (
                address[] addrs, ConsensusKeys[] keys, uint256[] stakes, bool[] tombstoned);
        function getRegistryWithKeys()
            external view returns (address[] addrs, ConsensusKeys[] keys);
        // Committee-change bit: set DETERMINISTICALLY by the contract at
        // `commitEpochCommittee` (`dkgQual[epoch] = committee[epoch] != committee[epoch−1]`),
        // NOT via a permissionless marker tx. `true` ⇒ the committee changed at
        // `epoch` (its DKG re-mints the beacon key); `false` ⇒ unchanged (carry
        // forward). Consumed by `beacon::carry` as the carry-forward arbiter.
        function getDkgQual(uint64 epoch) external view returns (bool);

        // Chain-configuration views. Formerly a separate `ChainConfig` predeploy;
        // same contract now, so the same address.
        function getEpochBlockInterval() external view returns (uint32);
        function getDposActivationBlock() external view returns (uint64);
        function getUndelegatePeriod() external view returns (uint32);
        function getActiveValidatorsLength() external view returns (uint32);
    }
}

/// Smallest committee the contract will commit: `commitEpochCommittee` reverts
/// `ERR_COMMITTEE_TOO_SMALL` below it (`consensus.rs:523-529`), and
/// `setActiveValidatorsLength` refuses to store a cap under it
/// (`config.rs:499-503`), so a non-empty committee shorter than this cannot be a
/// legal on-chain state.
///
/// MUST mirror the staking contract's `MIN_COMMITTEE_LENGTH` (`consts.rs:388`).
pub const MIN_COMMITTEE_LENGTH: usize = 4;

/// Mirrors the staking contract's `BALANCE_COMPACT_PRECISION` (`consts.rs:336`,
/// `1e10`): on-chain `totalDelegated` is returned wei-scale (compacted ×1e10 by
/// `math::expand_balance`). The elector needs only relative weights, so we scale
/// back to the compacted `uint112` (fits `u128`). MUST mirror the contract —
/// drift mis-weights leaders.
pub const BALANCE_COMPACT_PRECISION: u128 = 10_000_000_000;

/// Upper bound on a compacted stake weight: the contract stores `totalDelegated`
/// compacted in a `uint112`, so any value at or above `2^112` is not a legal
/// on-chain state and must be rejected rather than carried into the elector.
///
/// This is the bound `WeightedVrf::build` cites when it argues its prefix-sum
/// accumulator cannot overflow (51 members × `< 2^112` ≈ `2^119` ≪ `u128::MAX`).
/// Before this constant existed that argument rested on the contract alone —
/// this function accepted anything up to `u128::MAX` — so the one invariant the
/// elector's safety depends on was documented on the Rust side and enforced only
/// on the Solidity side. Keep the two in step.
const MAX_COMPACT_STAKE: u128 = 1 << 112;

/// Wei-scale `totalDelegated` → compacted `uint112` weight (`u128`). Delegations
/// are exact multiples of [`BALANCE_COMPACT_PRECISION`] (`math::compact_balance`
/// rejects a remainder), so the division is lossless; `try_from` guards the
/// impossible `> u128` case and [`MAX_COMPACT_STAKE`] guards the `uint112` range
/// the elector's overflow argument depends on.
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

/// A validator's consensus identity, decoded and validated.
///
/// `bls_pubkey` is subgroup-checked on decode; `peer_pubkey` is a 32-byte
/// ed25519 key. Order in any `Vec` is **contract order, verbatim** — this crate
/// never sorts, and for a committee snapshot [`check_committee_ordering`] has
/// proved that order strictly ascending on the raw `peer_pubkey` bytes rather
/// than assuming it. Stake is NOT a key property — it lives on
/// [`ValidatorWithKeys::stake`] (the per-epoch frozen leader weight).
#[derive(Clone, Debug)]
pub struct ConsensusKeys {
    pub bls_pubkey: BlsPubkey,
    pub peer_pubkey: PeerPubkey,
    pub activation_epoch: u64,
}

/// A validator address paired with its consensus keys.
#[derive(Clone, Debug)]
pub struct ValidatorWithKeys {
    pub address: Address,
    pub keys: ConsensusKeys,
    /// The committee member's LEADER WEIGHT, compacted
    /// (`totalDelegated / BALANCE_COMPACT_PRECISION`). Leader weight only; NOT
    /// voting power.
    ///
    /// Literally frozen since 2026-07-31: stamped into `leaderStakes[epoch]` at
    /// `commitEpochCommittee` from the SELECTION epoch (`epoch − 2`) — the same
    /// vintage that ranked the membership — so it is byte-identical across nodes
    /// no matter which block hash each one reads at. It previously came from a
    /// live at-or-before walk, which made "same frozen source as committee
    /// selection" false: same helper, different epoch argument.
    ///
    /// Consequence of that vintage worth knowing: a member whose stake was not yet
    /// effective at the selection epoch carries weight 0 for that epoch — it is in
    /// the committee (an under-full committee does not filter by stake) but cannot
    /// be elected leader and earns no stipend share for it.
    pub stake: u128,
    /// Whether this member has been slashed for equivocation.
    ///
    /// The ONE field of the snapshot that is NOT frozen at the epoch commit: it is
    /// read live at the snapshot's own block, because a verdict landing mid-epoch
    /// has to reach the committee it names. So unlike membership, keys and stake,
    /// this field is NOT hash-invariant — two nodes reading the same epoch at
    /// different heights can legitimately disagree for the few blocks it takes the
    /// verdict to reach them both. It is monotone (a tombstone is permanent on
    /// chain), so the disagreement only ever resolves in one direction and never
    /// oscillates. Do NOT build anything that needs byte-identical snapshots
    /// across nodes on it.
    pub tombstoned: bool,
}

/// Validator set as read at one specific block. `epoch` is computed locally
/// from `block_number` (see [`epoch_of_block`]), never via an `eth_call`.
#[derive(Clone, Debug)]
pub struct ValidatorSetSnapshot {
    pub block_hash: B256,
    pub block_number: u64,
    pub epoch: u64,
    pub validators: Vec<ValidatorWithKeys>,
}

/// Startup configuration. The staking address is not pinned in-tree; it arrives
/// in a JSON file distributed with the bootnode IP list (the genesis tooling owns
/// that file; this layer only parses it).
///
/// ONE address, deliberately, and with NO serde default. The registry, the epoch
/// committee, the chain-configuration views and the liveness recorder are one
/// rWasm contract, so three fields obliged to be equal would only be a sync
/// hazard — and its failure mode is the worst available: an omitted field falls
/// back to an address with no code, an EVM call to a codeless account returns
/// Success, and the system call becomes a silent no-op. A missing field must fail
/// the parse, so there is nothing to omit.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct StakingReaderConfig {
    /// The staking system contract.
    pub staking_address: Address,
}

impl StakingReaderConfig {
    /// Parse the JSON config file at `path`.
    pub fn from_json_path(path: &std::path::Path) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        serde_json::from_slice(&bytes)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}

/// Relative DPoS epoch: `(block_number - dpos_activation_block) / epoch_block_interval`
/// (integer division, matching the contract's `math::epoch_at_block`,
/// `math.rs:49-57`). `dpos_activation_block` is the `uint64` from
/// `getDposActivationBlock()` — zero ⇒ absolute numbering.
/// `epoch_block_interval` is the `uint32` from `getEpochBlockInterval()`.
///
/// `saturating_sub` mirrors the contract's `block.number < activation ⇒ 0` clamp
/// (pre-activation blocks all map to epoch 0).
///
/// Caller MUST ensure `epoch_block_interval > 0` (it is governance-mutable
/// on-chain): `EpochTransition::on_finalized` and the dpos cold-start both
/// guard it. A zero here is a divide-by-zero panic.
#[inline]
pub fn epoch_of_block(
    block_number: u64,
    epoch_block_interval: u32,
    dpos_activation_block: u64,
) -> u64 {
    block_number.saturating_sub(dpos_activation_block) / epoch_block_interval as u64
}

/// Activation-relative epoch-boundary predicate: `true` when `block_number` is
/// the LAST block of its relative epoch, i.e. `(number + 1 - activation)` is a
/// multiple of `interval`. Activation-relative to match [`epoch_of_block`] and
/// the consensus `OriginEpocher` (an absolute `(number+1) % interval` check only
/// agrees when `activation % interval == 0`). Single definition shared by
/// `EpochTransition`'s frozen-geometry (`is_epoch_boundary_frozen`) and in-flight
/// boundary checks (`apply_at`) — so no caller hand-rolls the formula.
///
/// A PRE-ACTIVATION block belongs to no relative epoch (bug 3): it returns
/// `false` for every `block_number + 1 <= dpos_activation_block`, matching
/// `OriginEpocher::containing → None` — the two epoch authorities must agree, and
/// the earlier `saturating_sub` underflow to `0` wrongly classified EVERY
/// pre-activation block as a boundary (spurious cold-start / catch-up on the live
/// sequencer→DPoS migration path).
#[inline]
pub fn is_epoch_boundary(
    block_number: u64,
    epoch_block_interval: u32,
    dpos_activation_block: u64,
) -> bool {
    // A PRE-activation block (`block_number < activation`, i.e. `number + 1 <=
    // activation`, incl. block `activation - 1` whose rel would be 0) belongs to
    // no relative epoch. `activation == 0` (absolute numbering / mocks) is
    // unaffected: no block is `< 0`.
    if block_number < dpos_activation_block {
        return false;
    }
    (block_number + 1 - dpos_activation_block).is_multiple_of(epoch_block_interval as u64)
}

/// Tracker-feed guard: the peer set fed to `Oracle::track` (the Active
/// validator registry ∪ current committee) must fit commonware's
/// `max_peer_set_size`, or `track` panics deep in the p2p actor
/// (`tracker/actor.rs:158-163`). Call this at the epoch boundary *before*
/// `track` for an actionable error + a single controlled failure mode
/// instead of an opaque panic.
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

/// The committee's order IS the consensus index space, so assert it on arrival.
///
/// The node writes a commonware `Participant` index into the block (`extra_data`
/// `accused`) and the contract resolves the accused positionally from its own
/// stored array. The two agree only while both order by ascending raw
/// peer-pubkey bytes — the contract sorts on `peer_pubkey: B256` whose derived
/// `Ord` is byte-lex (`consensus.rs:538`), commonware's `BiMap` sorts on
/// `PeerPubkey: Ord` which is the same comparator. Nothing else asserts that
/// agreement across the two repositories, and a drift slashes the wrong
/// validator in silence, so this is a hard error rather than a warning. Consumers
/// treat a [`ReadError`] as deferrable, never fatal-to-process (`on_finalized`
/// logs + degrades a gauge, the slasher buffers a transient), so failing the read
/// costs a retry, not a halt.
///
/// Two properties are load-bearing and must not be "simplified":
/// - it compares **raw bytes** (`as_ref()`), not `PeerPubkey: Ord`, because raw
///   byte-lex is the comparator the *contract* uses and that is the side of the
///   agreement this check exists to anchor. `tests::peer_pubkey_ord_is_raw_byte_lex`
///   closes the loop to commonware's comparator;
/// - an **empty** committee is NOT an error. Six call sites branch on
///   `validators.is_empty()` as a routine deferral (an uncommitted / not-yet-read
///   epoch), and erroring here would turn the epoch-boundary park into a degraded
///   retry loop.
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

/// Decode one ABI `ConsensusKeys` tuple into the validated reader type.
/// Keys go through the subgroup-checked `fluentbase-bls` decoders (the same
/// path the consensus layer trusts) so a malformed 96-byte blob is rejected
/// here, never propagated. An *unset* entry (`blsPubkey.len() == 0`) is NOT
/// a valid `ConsensusKeys` — check [`is_unset`] first.
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

/// One read-only system call to `addr` with `calldata` against an
/// already-built EVM. View functions do not mutate, so the returned state
/// delta is discarded (`transact_system_call` never commits — the next call
/// on the same `evm` reads the identical immutable block state). Uses the
/// system-call path (no caller funding / nonce / gas) — staking getters don't
/// gate on `msg.sender`.
///
/// Free fn over `&mut impl Evm` so it can run against either a one-shot EVM
/// (single getter) or a hoisted EVM reused for every member of a committee
/// snapshot (`epoch_committee_snapshot`) — the header/state are invariant at a
/// fixed `at`, so building them once per snapshot is the whole point.
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

/// ABI-encode `call`, run it against `evm` via [`exec_view`], and ABI-decode
/// the return — the `abi_encode → exec → abi_decode_returns → map_err` pipeline
/// the typed getters all share, collapsed to one site. The `sol!` call type
/// (`C`) and target `addr` are the only variation.
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
/// `epoch_block_interval` is NOT cached via `OnceLock`. It is
/// governance-mutable on-chain (`ChainConfig.setEpochBlockInterval`); caching
/// the first read forever produces a consensus split if governance ever
/// changes the value while nodes are live. Re-reading per call costs one
/// extra in-process EVM STATICCALL (~tens of µs) — negligible relative to
/// the blast radius. The Solidity-side immutability story is owned by the
/// staking contracts; this Rust mitigation works independently.
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

    /// Build the EVM for the state at block `at` ONCE and hand it to `f`. The
    /// header read + state-provider build + EVM construction are invariant at a
    /// fixed `at`, so a multi-call read (`epoch_committee_snapshot`) builds them
    /// a single time and reuses the `&mut Ev` for every member.
    ///
    /// This hoist is scoped to ONE `at` per invocation — it is NOT a persistent
    /// cross-read cache; the deliberate "no cross-call caching" invariant
    /// (governance-mutable params, reorg safety) is unaffected.
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

        // Hand the header to `f` too: a multi-call read (`epoch_committee_snapshot`)
        // gets the block number from it instead of a SECOND `provider.header(at)`.
        f(&mut evm, &header)
    }

    /// ABI-encoded typed read of `call` against `addr` at block `at`: the
    /// `abi_encode → exec_view → abi_decode_returns → map_err(AbiDecode)`
    /// boilerplate the typed getters share, collapsed behind one generic site.
    /// Builds a one-shot EVM via [`Self::with_evm`] (single top-level read);
    /// within a snapshot read the hoisted EVM is reused directly via
    /// [`decode_view`]. Uses the system-call path (no caller funding / nonce /
    /// gas) — staking getters don't gate on `msg.sender`.
    fn call<C: SolCall>(&self, addr: Address, call: &C, at: B256) -> Result<C::Return, ReadError> {
        self.with_evm(at, |evm, _header| decode_view(evm, addr, call))
    }

    /// `getEpochBlockInterval()` at block `at`.
    ///
    /// Re-read on every call (no cache). The cost is one in-process
    /// EVM STATICCALL per finalized block — negligible relative to a
    /// governance-flip consensus-split blast radius.
    pub fn epoch_block_interval(&self, at: B256) -> Result<u32, ReadError> {
        self.call(
            self.cfg.staking_address,
            &abi::getEpochBlockIntervalCall {},
            at,
        )
    }

    /// `getDposActivationBlock()` at block `at` — origin for the relative DPoS
    /// epoch numbering. `0` is the unscheduled sentinel
    /// (`setDposActivationBlock` requires a future block, so a live chain never
    /// stores `0`; cf. `crates/node/src/evm.rs`). Re-read per call.
    pub fn dpos_activation_block(&self, at: B256) -> Result<u64, ReadError> {
        self.call(
            self.cfg.staking_address,
            &abi::getDposActivationBlockCall {},
            at,
        )
    }

    /// Activation height as a *scheduling state*: `Ok(None)` while the staking
    /// contract has no code at `at` (not installed yet — the production-path
    /// smoke pre-writes the reader config before the runtime-upgrade delivery)
    /// or while activation is unscheduled (`0`); `Ok(Some(h))` once governance
    /// has scheduled it. The code-presence probe mirrors the executor's P2-2
    /// gate (`crates/node/src/evm.rs`) at the provider layer so launcher-side
    /// consumers can boot with a pre-written config. A raw
    /// [`Self::dpos_activation_block`] against a codeless account would
    /// instead surface as an `AbiDecode` error on the empty return.
    ///
    /// The probe and the read now target the SAME address, which is strictly
    /// tighter than before: it used to probe the `ChainConfig` predeploy for the
    /// sake of a call issued to that same predeploy, so the property it asserts
    /// ("the account serving `getDposActivationBlock` has code") is unchanged
    /// while the window where the two addresses could disagree is gone.
    pub fn scheduled_dpos_activation(&self, at: B256) -> Result<Option<u64>, ReadError> {
        let state = self
            .provider
            .state_by_block_hash(at)
            .map_err(map_state_provider_err)?;
        // reth normalizes no-code accounts to `bytecode_hash: None`; the
        // KECCAK_EMPTY arm is defensive against unnormalized providers.
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

    /// `getActiveValidatorsLength()`. Used at startup by the host adapter to
    /// enforce the Rust ↔ contract invariant
    /// `activeValidatorsLength <= fluentbase_p2p::constants::MAX_COMMITTEE_SIZE`.
    /// The value is bounded on-chain by `MAX_ACTIVE_VALIDATORS` (currently 51);
    /// if the two caps ever drift, the production record's wire format (u8
    /// leader_index) or scheme building would break — the startup assert catches
    /// this earlier with an actionable error pointing at both source files.
    pub fn active_validators_length(&self, at: B256) -> Result<u32, ReadError> {
        self.call(
            self.cfg.staking_address,
            &abi::getActiveValidatorsLengthCall {},
            at,
        )
    }

    /// `getDkgQual(epoch)` at block `at` — the on-chain committee-change
    /// bit for `epoch`. Set DETERMINISTICALLY by the contract at
    /// `commitEpochCommittee` (`dkgQual[epoch] = committee[epoch] != committee[epoch−1]`),
    /// NOT via a permissionless marker tx. `true` ⇒ the committee changed at `epoch`
    /// (its DKG re-mints the beacon key); `false` ⇒ unchanged (carry the prior key
    /// forward). Re-read per call, same read semantics as
    /// [`Self::epoch_committee_snapshot`]; consumed by `beacon::carry` as the
    /// carry-forward arbiter (immutable once the epoch's committee is committed).
    pub fn dkg_qual(&self, epoch: u64, at: B256) -> Result<bool, ReadError> {
        self.call(self.cfg.staking_address, &abi::getDkgQualCall { epoch }, at)
    }

    /// Snapshot of the **frozen `epoch` committee** (authoritative for the
    /// peer set / slashing window / leader weights), each member joined with
    /// its full consensus keys AND frozen effective stake, at block `at`. This
    /// is what the cache persists.
    ///
    /// One `getEpochCommitteeWithStakes` call returns the complete per-epoch
    /// snapshot — `(addrs, keys, stakes, tombstoned)` — keeping the full
    /// [`ConsensusKeys`] (bls + peer + activationEpoch) the codec needs plus the
    /// per-member [`ValidatorWithKeys::stake`] the leader elector consumes. The
    /// first three legs are frozen at the epoch commit; `tombstoned` is read live
    /// at `at` (see [`ValidatorWithKeys::tombstoned`]), which is what lets a
    /// mid-epoch verdict reach the committee it names. A keyless committee member ⇒
    /// [`ReadError::CommitteeMemberKeyless`] (on-chain invariant violation),
    /// never silently skipped. Empty / uncommitted epoch ⇒ a snapshot with
    /// `validators: []`. A length mismatch across the four arrays ⇒
    /// [`ReadError::AbiDecode`] (the contract returns equal-length arrays).
    ///
    /// This is the single site [`check_committee_ordering`] runs at, so every
    /// consumer inherits the index-space invariant through the one shared
    /// snapshot rather than each re-asserting it.
    pub fn epoch_committee_snapshot(
        &self,
        epoch: u64,
        at: B256,
    ) -> Result<ValidatorSetSnapshot, ReadError> {
        let staking = self.cfg.staking_address;
        let (block_number, validators) = self.with_evm(at, |evm, header| {
            // Block number from the already-read header — no second header read.
            let block_number = header.number();
            let ret = decode_view(
                evm,
                staking,
                &abi::getEpochCommitteeWithStakesCall { epoch },
            )?;
            if ret.addrs.len() != ret.keys.len()
                || ret.addrs.len() != ret.stakes.len()
                || ret.addrs.len() != ret.tombstoned.len()
            {
                return Err(ReadError::AbiDecode(
                    "committee/keys/stakes/tombstoned length mismatch".into(),
                ));
            }
            let validators = ret
                .addrs
                .into_iter()
                .zip(ret.keys)
                .zip(ret.stakes)
                .zip(ret.tombstoned)
                .map(|(((address, k), stake_wei), tombstoned)| {
                    if is_unset(&k) {
                        return Err(ReadError::CommitteeMemberKeyless {
                            epoch,
                            validator: address,
                        });
                    }
                    Ok(ValidatorWithKeys {
                        address,
                        keys: decode_consensus_keys(k)?,
                        stake: compact_stake(stake_wei)?,
                        tombstoned,
                    })
                })
                .collect::<Result<Vec<_>, ReadError>>()?;
            check_committee_ordering(epoch, &validators)?;
            Ok((block_number, validators))
        })?;
        Ok(ValidatorSetSnapshot {
            block_hash: at,
            block_number,
            epoch,
            validators,
        })
    }

    /// Peer keys of the FULL Active-status validator registry
    /// (`getRegistryWithKeys` = the active-validator list, NOT the
    /// stake-weighted top-k committee) at block `at`. Feeds the consensus
    /// p2p tier-2 peer set: every activated validator — in or out of the
    /// committee, including the sequencer — keeps consensus-plane
    /// connectivity. Keyless entries (registered but with no consensus keys
    /// set yet) are SKIPPED: unlike a committee member, a keyless registry
    /// entry is a legal transient state, not an invariant violation.
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

/// Trait-ified read surface over [`RethStakingStateReader`] — the exact subset
/// of staking reads the consensus layer consumes (the epoch-boundary
/// orchestrator `EpochTransition`, the slasher, and `OuterEngine`). Kept as a
/// trait so those consumers stay generic over the reader and can inject
/// deterministic mocks in tests; the production impl is the blanket one on
/// [`RethStakingStateReader`] below.
pub trait StakingStateRead {
    /// Frozen committee for `epoch` (+ full keys) at block `at`.
    fn epoch_committee_snapshot(
        &self,
        epoch: u64,
        at: B256,
    ) -> Result<ValidatorSetSnapshot, ReadError>;

    /// `ChainConfig.getEpochBlockInterval()` (blocks per epoch) at `at`.
    /// Read per call (no OnceLock cache).
    fn epoch_block_interval(&self, at: B256) -> Result<u32, ReadError>;

    /// `ChainConfig.getDposActivationBlock()` (relative-epoch origin) at `at`.
    fn dpos_activation_block(&self, at: B256) -> Result<u64, ReadError>;

    /// DPoS activation as a *scheduling state* — the codeless/unscheduled-tolerant
    /// gate the beacon-plane cold-start defers on (`None` ⇒ not yet a DPoS chain at
    /// `at`, so freezing geometry would read a reverting/empty ChainConfig).
    ///
    /// The default folds NOTHING (`Ok(Some(dpos_activation_block(at)?))`) so the
    /// in-memory test mocks — which always have a live synthetic ChainConfig, and
    /// legitimately use activation `0` for absolute numbering — stay "scheduled"
    /// and freeze exactly as before. The real [`RethStakingStateReader`] OVERRIDES
    /// this with a provider-level code-presence probe + the `0`-sentinel fold (see
    /// [`RethStakingStateReader::scheduled_dpos_activation`]); only there does a
    /// codeless / unscheduled anchor surface as `None`.
    ///
    /// `activation == 0` ⇒ "absolute numbering (legitimate)" is a TEST-MOCK-ONLY
    /// affordance and NOT a live-chain divergence: it is meaningful ONLY through
    /// this default impl (synthetic mocks with no real `setDposActivationBlock`
    /// constraint). On a REAL chain `0` can never be a genuine activation —
    /// `setDposActivationBlock` refuses to store `0` (it requires a future block),
    /// the real [`RethStakingStateReader::scheduled_dpos_activation`] folds the
    /// `0` sentinel to `None`, AND `dpos::resolve_cold_start_kind` FATALS on
    /// `activation == 0` ("the unscheduled sentinel"). So the apparent
    /// contradiction between "0 = absolute" here and "0 = unscheduled, fatal"
    /// there is intentional: the two layers describe different worlds (mock vs
    /// real), not the same one. Do NOT "reconcile" them by changing behavior.
    fn scheduled_dpos_activation(&self, at: B256) -> Result<Option<u64>, ReadError> {
        Ok(Some(self.dpos_activation_block(at)?))
    }

    /// Peer keys of the full Active validator registry (tier-2 feed),
    /// keyless-filtered. See [`RethStakingStateReader::active_registry_peers`].
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
    fn epoch_block_interval(&self, at: B256) -> Result<u32, ReadError> {
        RethStakingStateReader::epoch_block_interval(self, at)
    }
    fn dpos_activation_block(&self, at: B256) -> Result<u64, ReadError> {
        RethStakingStateReader::dpos_activation_block(self, at)
    }
    /// Override the trait default with the code-presence-aware probe: a codeless
    /// ChainConfig (runtime cluster not deployed at `at`) or an unscheduled `0`
    /// activation both map to `None`, so the beacon-plane cold-start can defer
    /// instead of fatally erroring on the empty/reverting read.
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
        abi, check_committee_ordering, check_peer_set_size, decode_consensus_keys, epoch_of_block,
        is_epoch_boundary, is_unset, map_evm_call_err, map_state_provider_err, StakingReaderConfig,
        ValidatorWithKeys, MIN_COMMITTEE_LENGTH,
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

    #[test]
    fn block_zero_is_epoch_zero() {
        assert_eq!(epoch_of_block(0, 100, 0), 0);
    }
    #[test]
    fn exact_multiple_advances_epoch() {
        assert_eq!(epoch_of_block(100, 100, 0), 1);
        assert_eq!(epoch_of_block(199, 100, 0), 1);
        assert_eq!(epoch_of_block(200, 100, 0), 2);
    }
    #[test]
    fn off_by_one_below_boundary_stays() {
        assert_eq!(epoch_of_block(99, 100, 0), 0);
    }
    #[test]
    fn relative_to_activation() {
        // activation=64, interval=32: anchor is relative epoch 0; advances every 32.
        assert_eq!(epoch_of_block(64, 32, 64), 0);
        assert_eq!(epoch_of_block(95, 32, 64), 0);
        assert_eq!(epoch_of_block(96, 32, 64), 1);
        assert_eq!(epoch_of_block(162, 32, 64), 3);
        // pre-activation clamps to epoch 0 (saturating_sub).
        assert_eq!(epoch_of_block(30, 32, 64), 0);
    }

    #[test]
    fn pre_activation_is_never_a_boundary() {
        // activation=64: every block with number+1 <= 64 is pre-activation and
        // must NOT be a boundary (the saturating_sub underflow used to make all
        // of them spurious boundaries — bug 3). Block activation-1 (== 63) too.
        for n in 0..64u64 {
            assert!(
                !is_epoch_boundary(n, 32, 64),
                "pre-activation block {n} must not be a boundary"
            );
        }
    }

    #[test]
    fn real_boundaries_are_at_activation_plus_k_interval_minus_one() {
        // The last block of relative epoch k is activation + k*interval - 1.
        for k in 1..5u64 {
            let last = 64 + k * 32 - 1;
            assert!(
                is_epoch_boundary(last, 32, 64),
                "block {last} is a boundary"
            );
            assert!(!is_epoch_boundary(last - 1, 32, 64));
            assert!(!is_epoch_boundary(last + 1, 32, 64));
        }
    }

    #[test]
    fn absolute_numbering_activation_zero_is_unchanged() {
        // activation=0 (mocks / absolute numbering): boundary at every interval-1.
        assert!(is_epoch_boundary(99, 100, 0));
        assert!(is_epoch_boundary(199, 100, 0));
        assert!(!is_epoch_boundary(0, 100, 0));
        assert!(!is_epoch_boundary(98, 100, 0));
        assert!(!is_epoch_boundary(100, 100, 0));
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

    /// The contract reports the tombstone positionally, so a decode that dropped
    /// or shifted the leg would name the wrong validator. Pinned against the
    /// `sol!` binding this crate calls the view through.
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

    /// `n` members whose raw peer-pubkey bytes ascend — the shape the contract
    /// commits (`consensus.rs:538` sorts on exactly this key) and the only shape
    /// the invariant accepts. Real ed25519 keys, because `PeerPubkey::decode`
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
                    stake: 0,
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

    /// The carve-out that makes the invariant deployable: an uncommitted epoch
    /// reads back `[]`, and six consumers branch on `validators.is_empty()` to
    /// park. Erroring on empty would turn the epoch-boundary park into a degraded
    /// retry loop. The carve-out is exactly *empty*, not *short*.
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

    /// Closes the loop on the index-space agreement. `check_committee_ordering`
    /// compares RAW BYTES because that is the contract's comparator (it sorts on
    /// `peer_pubkey: B256`, whose derived `Ord` is byte-lex); consumers index
    /// through commonware's `BiMap`, which orders by `PeerPubkey: Ord`. The
    /// positional slash is correct only while those two are the same order. They
    /// are today — ed25519 `PublicKey` derives `Ord` over `VerificationKey`,
    /// which compares its `[u8; 32]` — and this test is what fails on a
    /// commonware bump that changes it.
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

    /// Three `blsPubkey` blobs of three DIFFERENT lengths — 96 / 1 / 33 bytes,
    /// i.e. 3 / 1 / 2 words. Deliberate: a one-element array puts every head slot
    /// at offset 0, and equal-length elements give a uniform stride, so neither
    /// can catch a wrong head stride in an array of dynamic elements.
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

    /// Byte-level conformance for the four-array return, asserted in BOTH
    /// directions. This is the one layer in the migration where a wrong answer
    /// produces neither an exception nor a crash but plausible incorrect data:
    /// a head-stride defect shifts the keys against the addresses and the node
    /// signs on behalf of the wrong validator.
    ///
    /// The vector below is the NODE's four-array shape, generated independently
    /// with `cast abi-encode` — a real literal pin, not a re-encoding of the
    /// `sol!` under test. It is not what the contract emits today: see
    /// [`epoch_committee_return_arity_is_pinned`] for the three-vs-four drift.
    ///
    /// Vector produced by (0x…01/02/03 abbreviated, `0xa1`×96 / `0xb2` / `0xc3`×33,
    /// peer keys `0x11`×32 / `0x22`×32 / `0x33`×32):
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

    /// The hole a selector pin cannot cover: **return types do not enter a
    /// selector**, so `view_selectors_are_pinned` below agrees with the contract
    /// on `getEpochCommitteeWithStakes(uint64)` == `0xa4d160c1` while the two
    /// sides disagree about what comes back.
    ///
    /// KNOWN DRIFT, recorded not fixed (2026-08-14):
    ///
    /// * node     `returns (address[], ConsensusKeys[], uint256[], bool[] tombstoned)`
    ///   — the `sol!` above, four arrays.
    /// * contract `returns (address[], ConsensusKeys[], uint256[])` — three
    ///   arrays. `feat/flu-989-port-solidity-delta`, `contracts/staking/src/
    ///   consensus.rs`, `write_returns(sdk, &(validators, keys, stakes))`;
    ///   `origin/feat/flu-989-rust-staking` is identical.
    ///
    /// Reconciling needs the contract owner (the tombstone leg has no
    /// contract-side source yet), so this test only makes the shape explicit:
    /// the node's four-array return round-trips, the contract's three-array
    /// return does NOT decode into it, and the size difference is exactly the
    /// one extra `bool[]`. If someone adds or drops an array on either side,
    /// this fails and says which.
    #[test]
    fn epoch_committee_return_arity_is_pinned() {
        use alloy_sol_types::SolValue as _;

        let addr = Address::with_last_byte(1);
        let key = keys(11);
        let stake = U256::from(42u64);

        // What the NODE expects: four arrays.
        let node_shape = abi::getEpochCommitteeWithStakesCall::abi_encode_returns(
            &abi::getEpochCommitteeWithStakesReturn {
                addrs: vec![addr],
                keys: vec![key.clone()],
                stakes: vec![stake],
                tombstoned: vec![true],
            },
        );

        // What the CONTRACT returns today: three arrays. Encoded from the ABI
        // tuple directly, NOT through the `sol!` under test — the point is that
        // the two sides are asserted independently.
        let contract_shape = (vec![addr], vec![key], vec![stake]).abi_encode_params();

        assert_ne!(
            node_shape, contract_shape,
            "if these ever match, the drift recorded on this test is gone and the \
             doc comment above must go with it"
        );
        // Head offset word (32) + array length word (32) + one element (32).
        assert_eq!(
            node_shape.len(),
            contract_shape.len() + 96,
            "the node's return must exceed the contract's by exactly the extra \
             one-element `bool[] tombstoned` leg (offset + length + element)"
        );

        // The drift is a hard decode failure, not a silent mis-pairing: a node
        // pointed at the contract as it stands would error, not tombstone the
        // wrong validator.
        assert!(
            abi::getEpochCommitteeWithStakesCall::abi_decode_returns(&contract_shape).is_err(),
            "the contract's three-array return (feat/flu-989-port-solidity-delta) must not \
             decode into the node's four-array expectation — if it starts to, the fourth leg \
             is being silently mis-read and the tombstone would name the wrong validator"
        );

        // And the node's own shape survives the round trip with the leg intact.
        let ret = abi::getEpochCommitteeWithStakesCall::abi_decode_returns(&node_shape)
            .expect("the node's own four-array return must decode");
        assert_eq!(ret.addrs.len(), 1);
        assert_eq!(ret.keys.len(), 1);
        assert_eq!(ret.stakes, vec![stake]);
        assert_eq!(ret.tombstoned, vec![true]);
    }

    /// The rWasm contract dispatches on the raw 4-byte selector, so a signature
    /// typo here is not a mis-decode but an `ERR_UNKNOWN_METHOD` revert against a
    /// live chain. Values are the contract's own handler doc comments
    /// (`consensus.rs:309,584,683`, `config.rs:245,317,330,343`).
    ///
    /// Literal on purpose: `abi::…Call::SELECTOR` is the node's belief, the
    /// `hex!` is the contract's, and they are independent. A selector pin says
    /// nothing about return shape — see
    /// [`epoch_committee_return_arity_is_pinned`] for that half.
    #[test]
    fn view_selectors_are_pinned() {
        assert_eq!(
            abi::getEpochCommitteeWithStakesCall::SELECTOR,
            hex!("a4d160c1")
        );
        assert_eq!(abi::getRegistryWithKeysCall::SELECTOR, hex!("d96cbd7b"));
        assert_eq!(abi::getDkgQualCall::SELECTOR, hex!("2660899f"));
        assert_eq!(abi::getEpochBlockIntervalCall::SELECTOR, hex!("346c90a8"));
        assert_eq!(abi::getDposActivationBlockCall::SELECTOR, hex!("a2a50528"));
        assert_eq!(abi::getUndelegatePeriodCall::SELECTOR, hex!("5e7b72ad"));
        assert_eq!(
            abi::getActiveValidatorsLengthCall::SELECTOR,
            hex!("32cc6f08")
        );
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
        // The point of the collapse: there is no default to silently fall back to.
        // A defaulted address would land on a codeless account, and an EVM call to
        // a codeless account returns Success — a per-block silent no-op.
        assert!(serde_json::from_str::<StakingReaderConfig>(r#"{}"#).is_err());
    }

    #[test]
    fn state_not_found_maps_to_state_not_materialized() {
        // reth's typed state-miss (header present, executed state absent during
        // pipeline backfill) becomes the deferrable variant carrying the hash.
        let hash = B256::repeat_byte(0x7);
        match map_state_provider_err(ProviderError::StateForHashNotFound(hash)) {
            ReadError::StateNotMaterialized { hash: got } => assert_eq!(got, hash),
            other => panic!("expected StateNotMaterialized, got {other:?}"),
        }
    }

    #[test]
    fn other_provider_errors_keep_backend_mapping() {
        // Anything that is NOT a state-miss / torn read stays a `Backend` fault (fail-closed).
        assert!(matches!(
            map_state_provider_err(ProviderError::StateForNumberNotFound(42)),
            ReadError::Backend(_)
        ));
    }

    #[test]
    fn torn_changeset_decode_maps_to_transient_storage() {
        // A torn changeset ROW (the reth-fork `from_compact` panic replaced by a typed
        // `DatabaseError::Decode`) is a mid-append read, not corruption → deferrable.
        assert!(matches!(
            map_state_provider_err(ProviderError::Database(DatabaseError::Decode)),
            ReadError::TransientStorage(_)
        ));
    }

    #[test]
    fn torn_inconsistent_range_other_maps_to_transient_storage() {
        // A `NippyJar` inconsistent-range read arrives as `ProviderError::Other(AnyError)`
        // matched by the shared display constant (AnyError erases the typed chain link).
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
        // A non-torn `Other` (no torn display) is a genuine fault, not deferrable.
        let other = ProviderError::other(std::io::Error::other("disk on fire"));
        assert!(matches!(
            map_state_provider_err(other),
            ReadError::Backend(_)
        ));
    }

    /// Minimal stand-in mirroring `revm`'s `EVMError::Database(ProviderError)`
    /// `Error::source` impl (source → the wrapped DB error), so this exercises the
    /// exact chain-walk `map_evm_call_err` performs on a real EVM error.
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
        // The EVM error's `source()` chain preserves the typed `ProviderError`, so a torn
        // static-file read during EVM state execution is recovered TYPED, not by string.
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
