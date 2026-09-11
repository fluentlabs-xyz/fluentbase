//! Deterministic multi-node testbed (Э3.2, step 1 of `E3-2-STAND-RESEARCH.md` §6).
//!
//! N real [`crate::outer::OuterEngine`]s — marshal, executor, epoch manager,
//! per-epoch simplex engines — run under ONE `commonware_runtime::deterministic`
//! runner over ONE `commonware_p2p::simulated::Network`. Everything below the
//! consensus crate's seams is faked, and every fake is a lie about reth that the
//! conformance set (Э3.1) has to catch, not this module:
//!
//! * [`fakes::FakeChain`] / [`fakes::FakeDeriver`] / [`fakes::FakeBeacon`] —
//!   derive = seal a header whose identity is
//!   `keccak(order digest ‖ prev_randao(seed))`, so the hash depends on the
//!   agreed ordering block, which is what makes a cross-node hash comparison say
//!   something about consensus. The three share ONE [`fakes::FakeChain`] and
//!   split it into reth's two tiers: derive and `import_derived` land the block
//!   in a tree-private executed map (`InsertExecutedBlock`), and only
//!   `fork_choice_updated` makes it CANONICAL, i.e. visible to
//!   `spec_executed_hash` (= `provider.block_hash(n)`). The import always
//!   answers `Valid` (so does production's `InsertExecutedBlock` seam); the FCU
//!   answers `Valid` when it linked the head to the canonical chain and
//!   `SYNCING` when it could not, which is reth's missing-block branch and keeps
//!   the fake from ever writing a gapped or unlinked canonical map. The full
//!   list of what still diverges from reth — INVALID, the backfill-window
//!   SYNCING, the ignored safe/finalized tags, the eager sync-target
//!   canonicalization, the ancestor-FCU skip, engine-fatal errors, the by-hash
//!   index lag, `executed_tip`'s tier, no persistence — is the
//!   `/// Divergences from reth` block on [`fakes::FakeChain`], with a
//!   `.claude/RETH_INTERNALS.md` anchor and a reason per item. That block is the
//!   canonical home for the list; nothing else restates it.
//!   [`fakes::ElEvent`] is the ordering observable over the two tiers (every
//!   derive and every canonicalization, in order, per node —
//!   `stand::Outcome::el_events`): "the guard read `h` while `h` was still
//!   tree-only" is an index comparison, never an absent log line.
//! * [`fakes::FakeStaking`] — the `StakingStateRead` the production
//!   `EpochTransition` runs over (step 5): committees come from a per-epoch
//!   schedule the test writes down, and the membership is a pure function of the
//!   epoch — `at_hash` only decides whether the epoch is committed yet, never WHO
//!   is in it, so a committee read "at the landing hash" has the production
//!   call shape but not its state semantics (PLAN 4.0(а)).
//! * Randomness is either `StaticRandomness` (`Beacon::Static`: a fixed
//!   anonymous sharing, σ on demand, no DKG plane — the first two sessions'
//!   tests) or the REAL beacon plane (`Beacon::Live`, step 4): `beacon::build`
//!   on every node over `BEACON_CHANNEL` / `BEACON_RESOLVER_CHANNEL` of the
//!   consensus network and the same mux brokers, the live DKG, the epoch-key
//!   agreement instances, the key / seed / artifact journals under a per-node
//!   partition prefix, shares in a per-node directory under a real temp root.
//!   The staking reads the plane needs are ONE `beacon::CommitteeReads` impl
//!   (`read_at`, `committee`, `committee_bls`, `dkg_qual`) over the schedule,
//!   with the contract's rule `dkgQual[e] = committee[e] != committee[e-1]`.
//! * The upstream plane is REAL (step 3): every node runs the production
//!   frontier resolver + `PlaneUpstreamHandle` over `FRONTIER_CHANNEL` on a
//!   second simulated network, counted at both ends ([`fakes::UpstreamCounters`]),
//!   and the executor's frozen-tip probe is wired as in `dpos.rs::launch`. The
//!   steady-state re-jump is the PRODUCTION
//!   `cold_start_jump::cold_start_jump_with_threshold` (so `verify_jump_structural`
//!   and `verify_jump_authenticated` run for real), over two fakes for its two
//!   seams: [`fakes::JumpElSync`] for `RethElSync` (the EL peer is
//!   [`fakes::ElNetwork`]) and [`fakes::JumpCommittees`] for
//!   `RethCommitteeSource` (`committee[E]` read by EXECUTED HASH out of
//!   [`fakes::FakeStaking`]). Its gate is `u64::MAX` unless a test sets
//!   `StandConfig::re_jump_threshold`.
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
//! Lives in the crate as a `#[cfg(test)]` module, and every beacon it mounts is
//! an `Arc<dyn Beacon>` — the trait is the ONE substitution seam. The three
//! modes (`StaticRandomness`, the live plane, `absent` for `Role::AbsentBeacon`)
//! and the `WithholdingRandomness` wrapper all implement it directly, and they
//! come through `beacon::testing`, which is `#[cfg(test)]`: a separate crate
//! cannot reach any of them, so it cannot promote a node to `Signer`.

/// The byzantine wrappers of Э3.3 (`Role::TwoReveals`, `Role::ForgedSeedUpstream`,
/// `Role::InflatedProbe`, `Role::LyingUpstream`, `Role::WrongHeightFinalized`),
/// gated exactly as `crate::byzantine` is — a stand built without the feature has
/// no way to reach them.
#[cfg(feature = "dpos-devnet-byzantine")]
mod byzantine_roles;
mod capture;
mod committee_tests;
mod fakes;
mod preconditions;
mod stand;
mod tests;
