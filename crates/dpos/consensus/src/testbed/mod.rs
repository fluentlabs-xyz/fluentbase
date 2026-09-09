//! Deterministic multi-node testbed (Э3.2, step 1 of `E3-2-STAND-RESEARCH.md` §6).
//!
//! N real [`crate::outer::OuterEngine`]s — marshal, executor, epoch manager,
//! per-epoch simplex engines — run under ONE `commonware_runtime::deterministic`
//! runner over ONE `commonware_p2p::simulated::Network`. Everything below the
//! consensus crate's seams is faked, and every fake is a lie about reth that the
//! conformance set (Э3.1) has to catch, not this module:
//!
//! * [`fakes::FakeChain`] / [`fakes::FakeDeriver`] — derive = seal a header whose
//!   identity is `keccak(order digest ‖ prev_randao(seed))`, canonicalized AT
//!   DERIVE (the `FakeDeriver`-without-`land_on_import` model, R-006). The hash
//!   therefore depends on the agreed ordering block, which is what makes a
//!   cross-node hash comparison say something about consensus.
//! * [`fakes::FakeBeacon`] — every FCU / import is `Valid`.
//! * [`fakes::SnapshotReader`] — committees come from a per-epoch schedule the
//!   test writes down; the on-chain path (`EpochTransition` over a staking
//!   contract) is step 5 and NOT here. [`stand::Stand`] replays the schedule into
//!   the epoch manager with a boundary relay that mirrors
//!   `EpochTransition::on_finalized`'s cold-start + boundary arms.
//! * Randomness is `StaticRandomness` (beacon-INACTIVE epochs, no DKG plane —
//!   step 4).
//! * The upstream plane is REAL (step 3): every node runs the production
//!   frontier resolver + `PlaneUpstreamHandle` over `FRONTIER_CHANNEL` on a
//!   second simulated network, counted at both ends ([`fakes::UpstreamCounters`]),
//!   and the executor's frozen-tip probe is wired as in `dpos.rs::launch`; the
//!   re-jump itself (reth EL sync) is a counted no-op behind a `u64::MAX` gate.
//!
//! What the stand reports: executed heights and hashes per node, the first
//! height whose executed hashes disagree (a minority node, or a tie), what
//! each node's upstream plane asked and served, every node's
//! typed `SafetyHalt` reason, the `(height, view, leader, digest, hash)` trace of
//! every finalized block per node, and the WARN/ERROR log lines captured by
//! [`capture`]. Log lines carry NO node label: commonware attaches a tracing
//! span only to `traced` spawns and never to their children
//! (`runtime/src/deterministic.rs:1157-1173`), so attribution comes from the
//! typed per-node latch instead.
//!
//! Lives in the crate as a `#[cfg(test)]` module: `StaticRandomness` and the
//! `Randomness` trait's error types are `pub(crate)`, so a separate crate cannot
//! promote a node to `Signer`.

mod capture;
mod fakes;
mod stand;
mod tests;
