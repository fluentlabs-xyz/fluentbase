//! The ONE declaration of the Fluent staking contract's ABI.
//!
//! The staking module (`contracts/staking`) derives its dispatch selectors from
//! this crate; the node (`crates/node`, `crates/dpos`) builds its calldata and
//! decodes its returns and logs from the same declarations. Before this crate
//! existed each side wrote the signatures out independently — the contract as
//! `derive_keccak256_id!("…")` strings, the node as its own `sol!` blocks — and
//! the only thing holding them together was a hex literal transcribed out of a
//! doc comment. A rename on either side is now a compile error on the other.
//!
//! Scope: the surface the RUNNING node crosses. Handlers no node-side code calls
//! keep their own `derive_keccak256_id!` in the contract's `consts.rs`.
//!
//! That scope has a known hole, named here rather than left to be discovered:
//! `initialize` and `commitEpochCommittee` are declared a THIRD time, in
//! `devnet/local-dpos-smoke/genesis-bootstrap/src/bootstrap.rs`'s own `sol!`,
//! and `initialize` a fourth through sixth time across `e2e/src/staking*.rs`.
//! `bootstrap.rs` is not a test — it builds the genesis state a real stand runs
//! on, so an arity change to `initialize` compiles there and reverts at
//! genesis-init. Bringing it in is stand work, deliberately not done here.
//!
//! `no_std` and `alloy-sol-types` only, because the contract compiles to
//! `wasm32-unknown-unknown` with no `std`.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloy_sol_types::sol;
pub use alloy_sol_types::{SolCall, SolError, SolEvent, SolInterface, SolType};

sol! {
    /// A validator's consensus identity as the contract stores and returns it.
    ///
    /// `blsPubkey` is exactly 96 B when set (compressed BLS12-381 G2, MinSig)
    /// and EMPTY when unset — the contract's "no consensus keys yet" sentinel,
    /// a legal transient state in the registry and an invariant violation
    /// inside a committee.
    #[derive(Debug, PartialEq)]
    struct ConsensusKeys {
        bytes blsPubkey;
        bytes32 peerPubkey;
        uint64 activationEpoch;
    }

    // ---- system calls (pre-execution, SYSTEM_ADDRESS only) ------------------

    /// The ONE liveness system call: who produced this block.
    ///
    /// The height is NOT an argument — the contract reads `block.number` from
    /// its own context and uses it as both idempotency key and epoch cursor,
    /// because a height it derives cannot disagree with the block it executes
    /// in. `leaderIndex` is the asymmetric half, underivable on-chain, which is
    /// why it is verified at vote time instead. The epoch close — verdicts,
    /// exclusion stamps and the stipend settlement — is driven from inside the
    /// contract, so there is no second call.
    ///
    /// FAIL-LOUD on the node: a selector miss reverts `UnknownMethod()` into a
    /// block-execution error on every node at once.
    function recordProduction(uint8 leaderIndex) external;

    /// The epoch-committee freeze, system-caller only.
    ///
    /// It takes NO argument: the contract derives the committee itself and sorts
    /// it ascending by peer pubkey, which IS the consensus index space
    /// `leaderIndex` and the slash `signerIdx` are resolved against. FAIL-LOUD,
    /// like `recordProduction`.
    function commitEpochCommittee() external;

    /// The equivocation VERDICT, system-caller only.
    ///
    /// It carries no evidence and the contract verifies none: every committee
    /// member checked the charge against the evidence in the OrderBlock before
    /// voting for the block, so the chain applies what the committee already
    /// agreed. SOFT-failed on the node, so a drift here costs slashes silently
    /// rather than halting.
    ///
    /// `signerIdx` is a committee position — one byte on the wire, widened
    /// here, so the widening is total.
    function slashEquivocation(uint64 epoch, uint32 signerIdx) external;

    // ---- evidence transactions (ordinary txs from the slasher EOA) ----------

    function slashEquivocationNotarize(bytes evidence, bytes pkUncompressed,
        bytes sig1Uncompressed, bytes sig2Uncompressed) external;
    function slashEquivocationFinalize(bytes evidence, bytes pkUncompressed,
        bytes sig1Uncompressed, bytes sig2Uncompressed) external;
    function slashEquivocationNullifyFinalize(bytes evidence, bytes pkUncompressed,
        bytes sig1Uncompressed, bytes sig2Uncompressed) external;

    // ---- views the node reads ----------------------------------------------

    /// The frozen `epoch` committee, joined with keys, weights and the LIVE
    /// equivocation tombstone.
    ///
    /// The first three legs are frozen at the epoch commit; `tombstoned` is read
    /// live at the call's block, which is what lets a mid-epoch verdict reach
    /// the committee it names. `stakes` is the one leg allowed to come back
    /// EMPTY beside a non-empty `addrs` — that is how the contract says its
    /// weight ring has wrapped past this epoch.
    function getEpochCommitteeWithStakes(uint64 epoch)
        external view returns (
            address[] addrs, ConsensusKeys[] keys, uint256[] stakes, bool[] tombstoned);

    /// The FULL Active-status validator registry — not the stake-weighted
    /// committee. Feeds the consensus p2p tier-2 peer set.
    function getRegistryWithKeys()
        external view returns (address[] addrs, ConsensusKeys[] keys);

    /// Committee-change bit, set DETERMINISTICALLY inside `commitEpochCommittee`
    /// (`dkgQual[epoch] = committee[epoch] != committee[epoch−1]`), not by a
    /// permissionless marker tx. `true` ⇒ the committee changed at `epoch` and
    /// its DKG re-mints the beacon key.
    function getDkgQual(uint64 epoch) external view returns (bool);

    /// The next-uncommitted epoch — the commit cursor.
    ///
    /// With `commitEpochCommittee` argument-free this is the only node-side
    /// evidence that a commit did anything: the ahead-commit loop's termination
    /// and its stuck-cursor guard both rest on it.
    function nextEpochToCommit() external view returns (uint64);

    // Chain-configuration views. Formerly a separate `ChainConfig` predeploy;
    // the same contract now, so the same address.
    //
    // `uint64`, not `uint32`: the contract stores all three as `u64` and
    // `write_abi`s them at that width. The node used to declare `uint32` and
    // got away with it only because the values happen to fit.
    function getEpochBlockInterval() external view returns (uint64);
    function getDposActivationBlock() external view returns (uint64);
    function getActiveValidatorsLength() external view returns (uint64);

    // ---- events the node decodes -------------------------------------------
    //
    // These ride the logs of PRE-EXECUTION system calls, which never become
    // receipts — `eth_getLogs` shows nothing of them — so decoding them in the
    // executor is the only way anyone ever sees them.

    /// The partial-epoch taint is DERIVED from the recorded block count, so ONE
    /// unrecorded block silently costs a whole epoch its verdicts while the tier
    /// still reads as enabled. Nothing else says so.
    event PartialEpoch(uint64 indexed epoch, uint32 recorded, uint32 expected);
    /// The weight ring wrapped past `epoch`: it can be neither judged nor paid.
    /// On chain the outcome is byte-identical to an epoch that legitimately owed
    /// nothing, so this event is the only thing that tells them apart.
    event EpochWeightsUnavailable(uint64 indexed epoch, uint32 members);
    event ProductionVerdictFailed(
        uint64 indexed epoch,
        address indexed validator,
        uint32 produced,
        uint256 due
    );
    event CorrelatedFailureEpoch(uint64 indexed epoch, uint256 newFailures, uint256 tolerance);
    /// What the epoch close decided the epoch owes its committee.
    event EpochBlendRewardsCommitted(uint64 indexed epoch, uint256 blendAmount);
    /// The ordinary commit outcome. The committee is frozen two epochs ahead, so
    /// the value belongs to epoch `current + MAX_COMMITTEE_LOOKAHEAD_EPOCHS`.
    event EpochCommitteeCommitted(uint64 indexed epoch, address[] committee);

    // ---- reverts the node classifies ---------------------------------------

    /// The equivocation replay guard. The slasher's pre-flight simulation
    /// matches this selector to recognise an already-tombstoned victim and ack
    /// its WAL entry without spending gas.
    error AlreadySlashedForEquivocation(address validator);
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_sol_types::{SolCall, SolError, SolEvent};

    /// The thirteen CALL selectors this crate hands to both sides, pinned against
    /// the hex in `devnet/local-dpos-smoke/contracts/STAKING_ARTEFACT.md` — the
    /// scan of the deployed rWasm blob, which is an EXTERNAL witness: it is
    /// derived from neither declaration, it is what a live chain actually
    /// dispatches on, and it is re-run against every rebuilt blob.
    ///
    /// The fourteenth entry, `AlreadySlashedForEquivocation`, is NOT in that
    /// scan and cannot be: the scan walks handler selectors, and an error
    /// selector never appears as one. Its independent witness is
    /// `e2e/src/staking_bls.rs`, which asserts these four bytes coming back from
    /// a real rWasm revert.
    ///
    /// This is the one pin worth having. Recomputing a selector from the
    /// signature string next to it would put both halves on the same side, which
    /// is exactly the class of test this crate exists to retire.
    #[test]
    fn selectors_match_the_deployed_artefact_scan() {
        for (name, actual, pinned) in [
            (
                "recordProduction(uint8)",
                recordProductionCall::SELECTOR,
                0x1752910eu32,
            ),
            (
                "commitEpochCommittee()",
                commitEpochCommitteeCall::SELECTOR,
                0xe505b249,
            ),
            (
                "slashEquivocation(uint64,uint32)",
                slashEquivocationCall::SELECTOR,
                0xdc6fb3f2,
            ),
            (
                "slashEquivocationNotarize(bytes,bytes,bytes,bytes)",
                slashEquivocationNotarizeCall::SELECTOR,
                0xe28d2f63,
            ),
            (
                "slashEquivocationFinalize(bytes,bytes,bytes,bytes)",
                slashEquivocationFinalizeCall::SELECTOR,
                0xadd07a3e,
            ),
            (
                "slashEquivocationNullifyFinalize(bytes,bytes,bytes,bytes)",
                slashEquivocationNullifyFinalizeCall::SELECTOR,
                0xa10827e9,
            ),
            (
                "getEpochCommitteeWithStakes(uint64)",
                getEpochCommitteeWithStakesCall::SELECTOR,
                0xa4d160c1,
            ),
            (
                "getRegistryWithKeys()",
                getRegistryWithKeysCall::SELECTOR,
                0xd96cbd7b,
            ),
            ("getDkgQual(uint64)", getDkgQualCall::SELECTOR, 0x2660899f),
            (
                "nextEpochToCommit()",
                nextEpochToCommitCall::SELECTOR,
                0xc06a82de,
            ),
            (
                "getEpochBlockInterval()",
                getEpochBlockIntervalCall::SELECTOR,
                0x346c90a8,
            ),
            (
                "getDposActivationBlock()",
                getDposActivationBlockCall::SELECTOR,
                0xa2a50528,
            ),
            (
                "getActiveValidatorsLength()",
                getActiveValidatorsLengthCall::SELECTOR,
                0x32cc6f08,
            ),
            (
                "AlreadySlashedForEquivocation(address)",
                AlreadySlashedForEquivocation::SELECTOR,
                0x8300031d,
            ),
        ] {
            assert_eq!(
                u32::from_be_bytes(actual),
                pinned,
                "{name} drifted from the selector the deployed blob dispatches on"
            );
        }
    }

    /// The six event topic0s, pinned the same way. An event carries no selector
    /// into the blob scan, so the witness here is the contract's own derived
    /// `SELECTOR` constant read off `contracts/staking/src/events.rs` — and the
    /// contract asserts the other direction itself
    /// (`close_event_topics_match_the_shared_abi`), so the loop is closed on
    /// both sides rather than inside one of them.
    #[test]
    fn event_topics_are_pinned() {
        use alloy_sol_types::private::B256;
        for (name, actual, pinned) in [
            (
                "PartialEpoch",
                PartialEpoch::SIGNATURE_HASH,
                "0e3a2b176af3f126559b647eec7cf85052c5cf6239a5fb6869c57d8416225690",
            ),
            (
                "EpochWeightsUnavailable",
                EpochWeightsUnavailable::SIGNATURE_HASH,
                "fa53988f851fb1046eac62a82f8972150261b00d98f926786ce3e645b7991faf",
            ),
            (
                "ProductionVerdictFailed",
                ProductionVerdictFailed::SIGNATURE_HASH,
                "4d49874ac1e640f94f7ab435dd2305f79014052c7c659aac7f36742d043f8bd8",
            ),
            (
                "CorrelatedFailureEpoch",
                CorrelatedFailureEpoch::SIGNATURE_HASH,
                "3a7c10dc4c9367950614ebeed14db659b710db26c303e92aaf4ac4cdd5b10925",
            ),
            (
                "EpochBlendRewardsCommitted",
                EpochBlendRewardsCommitted::SIGNATURE_HASH,
                "c894e3fc351de77417f62399a4cb385021b2d2909a716569846a70cc8b9febc8",
            ),
            (
                "EpochCommitteeCommitted",
                EpochCommitteeCommitted::SIGNATURE_HASH,
                "015ffbf030c2f06f58cedc968ae2ec9df38a79be1a74f68686ca971ce1994a5d",
            ),
        ] {
            let want: B256 = pinned.parse().expect("pinned topic0 must be 32 hex bytes");
            assert_eq!(actual, want, "{name} topic0 drifted");
        }
    }
}
