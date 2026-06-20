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
use alloy_primitives::{address, Address, Bytes, B256};
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
    AccountReader, HeaderProvider, StateProviderBox, StateProviderFactory,
};

use crate::error::ReadError;

/// Solidity ABI subset this layer calls (verified against
/// `solidity-contracts`: `IStaking.sol:92-96` `ConsensusKeys`, `:231-245`
/// views; `IChainConfig.sol:41` `getEpochBlockInterval` — note `uint32`).
///
/// Kept as an inner module so the Solidity `ConsensusKeys` tuple does not
/// collide with the hybrid [`ConsensusKeys`] below (same identifier,
/// different types).
mod abi {
    use alloy_sol_types::sol;

    sol! {
        /// Mirrors `IStaking.ConsensusKeys`. `blsPubkey` is exactly 96 B when
        /// set (compressed BLS12-381 G2, MinSig); empty when unset.
        #[derive(Debug)]
        struct ConsensusKeys {
            bytes blsPubkey;
            bytes32 peerPubkey;
            uint64 activationEpoch;
        }

        // Staking contract
        function getConsensusKeys(address validator)
            external view returns (ConsensusKeys);
        function getEpochCommittee(uint64 epoch) external view returns (address[]);
        function getRegistryWithKeys()
            external view returns (address[] addrs, ConsensusKeys[] keys);

        // ChainConfig contract (separate address)
        function getEpochBlockInterval() external view returns (uint32);
        function getDposActivationBlock() external view returns (uint64);
        function getUndelegatePeriod() external view returns (uint32);
        function getActiveValidatorsLength() external view returns (uint32);
    }
}

/// On-chain `Staking.sol` epoch-committee retention margin
/// (`EPOCH_COMMITTEE_RETENTION_MARGIN`, `Staking.sol:54`): the contract
/// prunes committees older than `currentEpoch - (undelegatePeriod +
/// MARGIN)`. The cache mirrors this exact window (epoch_transition).
///
/// MUST mirror `solidity-contracts/contracts/staking/Staking.sol`
/// `EPOCH_COMMITTEE_RETENTION_MARGIN`. Any drift silently mis-prunes
/// the off-chain cache vs on-chain pruning — update both in the same PR.
pub const EPOCH_COMMITTEE_RETENTION_MARGIN: u64 = 8;

/// A validator's consensus identity, decoded and validated.
///
/// `bls_pubkey` is subgroup-checked on decode; `peer_pubkey` is a 32-byte
/// ed25519 key. Order in any `Vec` is **contract order, verbatim** — this
/// crate never sorts. `stake_weight` is intentionally absent.
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

/// Startup configuration. The staking + `ChainConfig` addresses are not
/// pinned in-tree; they arrive in a JSON file distributed with the bootnode
/// IP list (the genesis tooling owns that file; this layer only parses it).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct StakingReaderConfig {
    /// Staking system/predeploy contract address.
    pub staking_address: Address,
    /// `ChainConfig` system contract address (separate contract — what
    /// `Staking._currentEpoch()` dereferences for `epochBlockInterval`).
    pub chain_config_address: Address,
    /// `LivenessSlashing` contract address the executor system-calls for
    /// `processBitmap`. Defaults to the canonical predeploy slot so existing
    /// genesis-baked configs (which omit the field) keep working.
    #[serde(default = "default_liveness_slashing_address")]
    pub liveness_slashing_address: Address,
}

/// Mirror of `fluentbase_types::PRECOMPILE_LIVENESS_SLASHING`. Inlined (not
/// imported) to avoid adding a `fluentbase-types` dep to this crate; a
/// conformance test in `crates/node` (which depends on both) pins the equality.
fn default_liveness_slashing_address() -> Address {
    address!("0x0000000000000000000000000000000000520020")
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
/// (integer division, matching the contract's relative `_currentEpoch`,
/// `Staking.sol:400`). `dpos_activation_block` is the `uint64` from
/// `ChainConfig.getDposActivationBlock()` — zero ⇒ absolute numbering.
/// `epoch_block_interval` is the `uint32` from `ChainConfig.getEpochBlockInterval()`.
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
/// `EpochTransition`'s frozen-geometry and in-flight boundary checks, and
/// (`pub`, like its sibling [`epoch_of_block`]) by the follower-promotion
/// anchor gate in `crates/node` — so no caller hand-rolls the formula.
#[inline]
pub fn is_epoch_boundary(
    block_number: u64,
    epoch_block_interval: u32,
    dpos_activation_block: u64,
) -> bool {
    (block_number + 1)
        .saturating_sub(dpos_activation_block)
        .is_multiple_of(epoch_block_interval as u64)
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
        .map_err(|e| ReadError::Backend(e.to_string()))?;

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
/// `epoch_block_interval` and `undelegate_period` are NO LONGER
/// cached via `OnceLock`. Both are governance-mutable on-chain
/// (`ChainConfig.setEpochBlockInterval` / `setUndelegatePeriod`); caching
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
            .map_err(|e| ReadError::Backend(e.to_string()))?;

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

    /// `ChainConfig.getEpochBlockInterval()` at block `at`.
    ///
    /// Re-read on every call (no cache). The cost is one in-process
    /// EVM STATICCALL per finalized block — negligible relative to a
    /// governance-flip consensus-split blast radius.
    pub fn epoch_block_interval(&self, at: B256) -> Result<u32, ReadError> {
        self.call(
            self.cfg.chain_config_address,
            &abi::getEpochBlockIntervalCall {},
            at,
        )
    }

    /// `ChainConfig.getDposActivationBlock()` at block `at` — origin for the
    /// relative DPoS epoch numbering (zero ⇒ absolute). Re-read per call.
    pub fn dpos_activation_block(&self, at: B256) -> Result<u64, ReadError> {
        self.call(
            self.cfg.chain_config_address,
            &abi::getDposActivationBlockCall {},
            at,
        )
    }

    /// Activation height as a *scheduling state*: `Ok(None)` while the
    /// ChainConfig contract has no code at `at` (runtime cluster not deployed
    /// yet — the production-path smoke pre-writes the reader config before the
    /// forge deploy) or while activation is unscheduled (`0`); `Ok(Some(h))`
    /// once governance has scheduled it. The code-presence probe mirrors the
    /// executor's P2-2 gate (`crates/node/src/evm.rs`) at the provider layer
    /// so launcher-side consumers can boot with a pre-written config. A raw
    /// [`Self::dpos_activation_block`] against a codeless account would
    /// instead surface as an `AbiDecode` error on the empty return.
    pub fn scheduled_dpos_activation(&self, at: B256) -> Result<Option<u64>, ReadError> {
        let state = self
            .provider
            .state_by_block_hash(at)
            .map_err(|e| ReadError::Backend(e.to_string()))?;
        // reth normalizes no-code accounts to `bytecode_hash: None`; the
        // KECCAK_EMPTY arm is defensive against unnormalized providers.
        let deployed = state
            .basic_account(&self.cfg.chain_config_address)
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

    /// `ChainConfig.getUndelegatePeriod()` (epochs) at block `at`.
    ///
    /// Re-read on every call. Drives the epoch-committee retention
    /// window (`undelegatePeriod + EPOCH_COMMITTEE_RETENTION_MARGIN`) and
    /// mirrors the contract's own `_pruneStaleCommittees`.
    pub fn undelegate_period(&self, at: B256) -> Result<u32, ReadError> {
        self.call(
            self.cfg.chain_config_address,
            &abi::getUndelegatePeriodCall {},
            at,
        )
    }

    /// `ChainConfig.getActiveValidatorsLength()`. Used at startup by the host
    /// adapter to enforce the Rust ↔ Solidity invariant
    /// `activeValidatorsLength <= fluentbase_p2p::constants::MAX_COMMITTEE_SIZE`
    /// The value is bounded on-chain by `ChainConfig.MAX_ACTIVE_VALIDATORS`
    /// (currently 51); if Rust and Solidity caps ever drift, the attestation
    /// bitmap wire format (u8 committee_size) or scheme building would break —
    /// the startup assert catches this earlier with an actionable error
    /// pointing at both source files.
    pub fn active_validators_length(&self, at: B256) -> Result<u32, ReadError> {
        self.call(
            self.cfg.chain_config_address,
            &abi::getActiveValidatorsLengthCall {},
            at,
        )
    }

    /// Snapshot of the **frozen `epoch` committee** (authoritative for the
    /// peer set / slashing window), each member joined with its full
    /// consensus keys, at block `at`. This is what the cache persists —
    /// NOT the stake-DESC `getValidatorsWithKeys` candidate set (removed).
    ///
    /// One `getEpochCommittee` call + one `getConsensusKeys` per member —
    /// keeps the full [`ConsensusKeys`] (bls + peer + activationEpoch) the
    /// snapshot codec needs. A keyless committee member ⇒
    /// [`ReadError::CommitteeMemberKeyless`] (on-chain invariant violation),
    /// never silently skipped. Empty / uncommitted epoch ⇒ a snapshot with
    /// `validators: []`.
    ///
    /// All `1 + N` STATICCALLs run against a SINGLE EVM built once for `at`
    /// (via [`Self::with_evm`]) — the header read + state-provider build are
    /// invariant at a fixed `at`, so a 51-member committee no longer re-reads
    /// the header / rebuilds the state provider ~52 times. Each
    /// `transact_system_call` is a pure view that commits nothing, so reusing
    /// the EVM is byte-for-byte the same as the prior per-call construction
    /// (same calldata, same order, same decode, same error variants).
    pub fn epoch_committee_snapshot(
        &self,
        epoch: u64,
        at: B256,
    ) -> Result<ValidatorSetSnapshot, ReadError> {
        let staking = self.cfg.staking_address;
        let (block_number, validators) = self.with_evm(at, |evm, header| {
            // Block number from the already-read header — no second header read.
            let block_number = header.number();
            let committee: Vec<Address> =
                decode_view(evm, staking, &abi::getEpochCommitteeCall { epoch })?;
            let validators = committee
                .into_iter()
                .map(|address| {
                    let k = decode_view(evm, staking, &abi::getConsensusKeysCall { validator: address })?;
                    if is_unset(&k) {
                        return Err(ReadError::CommitteeMemberKeyless {
                            epoch,
                            validator: address,
                        });
                    }
                    Ok(ValidatorWithKeys {
                        address,
                        keys: decode_consensus_keys(k)?,
                    })
                })
                .collect::<Result<Vec<_>, ReadError>>()?;
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
    /// (`Staking.getRegistryWithKeys` = `_activeValidatorsList`, NOT the
    /// stake-weighted top-k committee) at block `at`. Feeds the consensus
    /// p2p tier-2 peer set: every activated validator — in or out of the
    /// committee, including the sequencer — keeps consensus-plane
    /// connectivity. Keyless entries (registered but `setConsensusKeys`
    /// not yet called) are SKIPPED: unlike a committee member, a keyless
    /// registry entry is a legal transient state, not an invariant
    /// violation.
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

    /// `ChainConfig.getUndelegatePeriod()` (epochs) at `at`.
    fn undelegate_period(&self, at: B256) -> Result<u32, ReadError>;

    /// `ChainConfig.getEpochBlockInterval()` (blocks per epoch) at `at`.
    /// Read per call (no OnceLock cache).
    fn epoch_block_interval(&self, at: B256) -> Result<u32, ReadError>;

    /// `ChainConfig.getDposActivationBlock()` (relative-epoch origin) at `at`.
    fn dpos_activation_block(&self, at: B256) -> Result<u64, ReadError>;

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
    fn undelegate_period(&self, at: B256) -> Result<u32, ReadError> {
        RethStakingStateReader::undelegate_period(self, at)
    }
    fn epoch_block_interval(&self, at: B256) -> Result<u32, ReadError> {
        RethStakingStateReader::epoch_block_interval(self, at)
    }
    fn dpos_activation_block(&self, at: B256) -> Result<u64, ReadError> {
        RethStakingStateReader::dpos_activation_block(self, at)
    }
    fn active_registry_peers(&self, at: B256) -> Result<Vec<PeerPubkey>, ReadError> {
        RethStakingStateReader::active_registry_peers(self, at)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        abi, check_peer_set_size, decode_consensus_keys, epoch_of_block,
        is_unset, StakingReaderConfig,
    };
    use crate::error::ReadError;
    use alloy_primitives::{address, Address, Bytes, FixedBytes};
    use alloy_sol_types::{SolCall, SolValue};
    use commonware_codec::Encode as _;
    use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer};
    use commonware_math::algebra::Random as _;
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng;

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
    fn empty_address_array_decodes_to_empty_vec() {
        let empty: Vec<Address> = vec![];
        let data = empty.abi_encode();
        let ret = abi::getEpochCommitteeCall::abi_decode_returns(&data)
            .expect("empty address[] must decode");
        assert!(ret.is_empty());
    }

    #[test]
    fn config_omitting_liveness_defaults_to_canonical_slot() {
        // Back-compat: genesis-baked configs predate the field and must still
        // land on the canonical predeploy slot (`PRECOMPILE_LIVENESS_SLASHING`).
        let json = r#"{
            "staking_address": "0x0000000000000000000000000000000000520010",
            "chain_config_address": "0x0000000000000000000000000000000000520011"
        }"#;
        let cfg: StakingReaderConfig = serde_json::from_str(json).expect("config must parse");
        assert_eq!(
            cfg.liveness_slashing_address,
            address!("0x0000000000000000000000000000000000520020")
        );
    }

    #[test]
    fn config_with_explicit_liveness_overrides_default() {
        let json = r#"{
            "staking_address": "0x0000000000000000000000000000000000520010",
            "chain_config_address": "0x0000000000000000000000000000000000520011",
            "liveness_slashing_address": "0x00000000000000000000000000000000000000ff"
        }"#;
        let cfg: StakingReaderConfig = serde_json::from_str(json).expect("config must parse");
        assert_eq!(
            cfg.liveness_slashing_address,
            address!("0x00000000000000000000000000000000000000ff")
        );
    }
}
