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
//! * Randomness is either `StaticRandomness` (`Beacon::Static`: a fixed
//!   anonymous sharing, σ on demand, no DKG plane — the first two sessions'
//!   tests) or the REAL beacon plane (`Beacon::Live`, step 4): `beacon::build`
//!   on every node over `BEACON_CHANNEL` / `BEACON_RESOLVER_CHANNEL` of the
//!   consensus network and the same mux brokers, the live DKG, the epoch-key
//!   agreement instances, the key / seed / artifact journals under a per-node
//!   partition prefix, shares in a per-node directory under a real temp root.
//!   The staking reads the plane needs (`committee_for`, `committee_pair_for`,
//!   `committee_source`, the `dkgQual` bit) come from the schedule, with the
//!   contract's rule `dkgQual[e] = committee[e] != committee[e-1]`.
//! * The upstream plane is REAL (step 3): every node runs the production
//!   frontier resolver + `PlaneUpstreamHandle` over `FRONTIER_CHANNEL` on a
//!   second simulated network, counted at both ends ([`fakes::UpstreamCounters`]),
//!   and the executor's frozen-tip probe is wired as in `dpos.rs::launch`; the
//!   re-jump itself (reth EL sync) is a counted no-op behind a `u64::MAX` gate.
//!
//! What the stand reports: executed heights and hashes per node, the first
//! height whose executed hashes disagree (a minority node, or a tie), what
//! each node's upstream plane asked and served, every node's
//! typed `SafetyHalt` reason, the `(height, view, leader, digest, hash, σ)`
//! trace of every finalized block per node, the σ each node's executor derived
//! every height from, the agreed epoch-key artifacts each node holds, the
//! runner's metrics text, and the WARN/ERROR log lines captured by
//! [`capture`]. Log lines carry NO node label: commonware attaches a tracing
//! span only to `traced` spawns and never to their children
//! (`runtime/src/deterministic.rs:1157-1173`), so attribution comes from the
//! typed per-node latch instead.
//!
//! Lives in the crate as a `#[cfg(test)]` module: `StaticRandomness` and the
//! `Randomness` trait's error types are `pub(crate)`, so a separate crate cannot
//! promote a node to `Signer`.

/// The byzantine wrappers of Э3.3 (`Role::TwoReveals`, `Role::ForgedSeedUpstream`),
/// gated exactly as `crate::byzantine` is — a stand built without the feature has
/// no way to reach them.
#[cfg(feature = "dpos-devnet-byzantine")]
mod byzantine_roles;
mod capture;
mod fakes;
mod stand;
mod tests;
