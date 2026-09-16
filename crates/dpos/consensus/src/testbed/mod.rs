//! Deterministic multi-node testbed.
//!
//! N real [`crate::outer::OuterEngine`]s — marshal, executor, epoch manager,
//! per-epoch simplex engines — run under one `commonware_runtime::deterministic`
//! runner over one `commonware_p2p::simulated::Network`. Everything below the
//! consensus crate's seams is faked, and every fake is a lie about reth that the
//! conformance set has to catch, not this module:
//!
//! * [`fakes::FakeChain`] / [`fakes::FakeDeriver`] / [`fakes::FakeBeacon`] —
//!   derive seals a header whose identity is
//!   `keccak(order digest ‖ prev_randao(seed))`, so the hash depends on the
//!   agreed ordering block, which is what makes a cross-node hash comparison say
//!   something about consensus. The three share one [`fakes::FakeChain`] and
//!   split it into reth's two tiers: derive and `import_derived` land the block
//!   in a tree-private executed map, and only `fork_choice_updated` makes it
//!   canonical, visible to `spec_executed_hash`. The import always answers
//!   `Valid`; the FCU answers `Valid` when it linked the head to the canonical
//!   chain and `SYNCING` when it could not, which keeps the fake from writing a
//!   gapped or unlinked canonical map. The list of what still diverges from reth
//!   is the `/// Divergences from reth` block on [`fakes::FakeChain`], with a
//!   reason per item; that block is the canonical home for the list.
//!   [`fakes::ElEvent`] is the ordering observable over the two tiers (every
//!   derive and every canonicalization, in order, per node —
//!   `stand::Outcome::el_events`).
//! * [`fakes::FakeStaking`] — the `StakingStateRead` the production
//!   `EpochTransition` runs over: committees come from a per-epoch schedule the
//!   test writes down, and the membership is a pure function of the epoch —
//!   `at_hash` only decides whether the epoch is committed yet, never who is in
//!   it, so a committee read "at the landing hash" has the production call shape
//!   but not its state semantics.
//! * Randomness is either `StaticRandomness` (`Beacon::Static`: a fixed
//!   anonymous sharing, σ on demand, no DKG plane) or the real beacon plane
//!   (`Beacon::Live`): `beacon::build` on every node over `BEACON_CHANNEL` /
//!   `BEACON_RESOLVER_CHANNEL` of the consensus network and the same mux brokers,
//!   the live DKG, the epoch-key agreement instances, the key / seed / artifact
//!   journals under a per-node partition prefix, shares in a per-node directory
//!   under a real temp root. The staking reads the plane needs are one
//!   `beacon::CommitteeReads` impl over the schedule, with the contract's rule
//!   `dkgQual[e] = committee[e] != committee[e-1]`.
//! * The upstream plane is real: every node runs the production frontier
//!   resolver + `PlaneUpstreamHandle` over `FRONTIER_CHANNEL` on a second
//!   simulated network, counted at both ends ([`fakes::UpstreamCounters`]), and
//!   the executor's frozen-tip probe is wired as in `dpos.rs::launch`. The
//!   steady-state re-jump is the production `cold_start_jump::jump_to_target`,
//!   over one fake for its remaining seam: [`fakes::JumpElSync`] for
//!   `RethElSync` (the EL peer is [`fakes::ElNetwork`]). Its gate is `u64::MAX`
//!   unless a test sets `StandConfig::re_jump_threshold`.
//!
//! What the stand reports: executed heights and hashes per node, the first
//! height whose executed hashes disagree, what each node's upstream plane asked
//! and served, every node's typed `SafetyHalt` reason, the
//! `(height, view, leader, digest, hash, σ)` trace of every finalized block per
//! node, the σ each node's executor derived every height from, the agreed
//! epoch-key artifacts each node holds, the runner's metrics text, and the
//! WARN/ERROR log lines captured by [`capture`]. Log lines carry no node label,
//! so attribution comes from the typed per-node latch instead.
//!
//! Lives in the crate as a `#[cfg(test)]` module, and every beacon it mounts is
//! an `Arc<dyn Beacon>` — the trait is the one substitution seam. The three modes
//! (`StaticRandomness`, the live plane, `absent` for `Role::AbsentBeacon`) and the
//! `WithholdingRandomness` wrapper all implement it directly, and they come
//! through `beacon::testing`, which is `#[cfg(test)]`: a separate crate cannot
//! reach any of them, so it cannot promote a node to `Signer`.

/// The byzantine wrappers (`Role::TwoReveals`, `Role::ForgedSeedUpstream`,
/// `Role::InflatedProbe`, `Role::LyingUpstream`, `Role::WrongHeightFinalized`),
/// gated exactly as `crate::byzantine` is — a stand built without the feature has
/// no way to reach them.
#[cfg(feature = "dpos-devnet-byzantine")]
mod byzantine_roles;
mod capture;
/// The stand's cert-inlet tests. A file of their own rather than more of
/// `tests.rs`, which is the main work surface.
mod cert_inlet_tests;
mod committee_tests;
mod fakes;
mod preconditions;
mod stand;
mod tests;
