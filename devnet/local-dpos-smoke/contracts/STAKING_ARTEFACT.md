# Staking rWasm artefact — provenance

Built from the sibling git worktree `/home/djadjka/Work/audit-482/pr482-study`
(branch `feat/flu-989-port-solidity-delta`), NOT from this tree. The contract
sources move here when those commits are squashed and merged; until then these
two blobs are vendored and must be rebuilt by hand when that branch moves.

Build command (run in the worktree root):

    cargo build --release -p fluentbase-genesis --features devnet-views

`devnet-views` is REQUIRED. Without it the artefact has no `producedAt`,
`blocksInEpoch`, `pendingExclusions` or `lastProcessedBlock`, and the harness's
production checks read the first two — a runtime failure on a live soak, not a
compile error. Verified: the feature adds 3,918 bytes (414,018 -> 417,936 for a
standalone contract build).

Where the outputs land (the build does NOT write here; copy them by hand):

    target/contracts/wasm32-unknown-unknown/release/fluentbase_contracts_staking.wasm
    target/release/build/fluentbase-genesis-<hash>/out/fluentbase_contracts_staking.rwasm

`<hash>` is cargo's build-script output directory and changes between builds;
pick the NEWEST one, and check its mtime against the build you just ran. There
are stale siblings from earlier builds beside it.

## Reproducibility — read this before trusting a blob

Nothing in the build hashes the sources into the artefact, and the artefact is
built from a DIRTY worktree, so "which sources produced this blob" is not
recoverable from the blob. The section below is the only record. When you rebuild,
replace it wholesale — HEAD, the dirty file list with its per-file source hashes,
and the resulting sizes and SHA-256s. A rebuild that does not update it makes the
gap wider, not the same.

Both blobs are **gitignored** (`.wasm` on `.gitignore:33`, `.rwasm` on `:34`), so git records nothing about
them at all — this file is the whole audit trail, and a rebuild that skips it
leaves no trace anywhere.

`.vendor-sha` beside them does NOT fingerprint this blob, despite what
`dpos_harness/stack/golden.py`'s docstring says ("a rebuilt staking module
invalidates the on-chain state"). The smoke `Makefile` writes the
**solidity-contracts** repo HEAD there, and a staking rebuild moves neither the
file nor that HEAD. What actually invalidates a golden snapshot after a rebuild
is `golden.py::_image_id` — the smoke docker image bakes the genesis, so the
image must be rebuilt before a golden is trusted. Rebuild the image after
swapping these blobs.

Cheap check that a blob carries the handlers this node calls: scan the `.rwasm`
for the LITTLE-ENDIAN selector words. (The `.wasm` encodes them as LEB128
constants, so a raw scan finds nothing there — scan the `.rwasm`.)

    python3 - <<'EOF'
    import pathlib
    b = pathlib.Path("fluentbase_contracts_staking.rwasm").read_bytes()
    for name, sel in [("recordProduction",0x1752910e),("commitEpochCommittee",0xe505b249),
                      ("slashEquivocation",0xdc6fb3f2),("producedAt",0x91c7d453)]:
        print(name, b.count(sel.to_bytes(4,"little")))
    EOF

All four must print `1`. A `0` for `producedAt` means the `devnet-views` feature
was missed.

The per-epoch beacon key (`commitEpochBeaconKey` `0x6ece9cb1` /
`getEpochBeaconKey` `0xc9adaf5c`) was added on 2026-08-16 and REMOVED again on
2026-08-17 together with the whole on-chain `PK_E` layer. A blob that still
contains either selector is the 2026-08-16 build and is STALE. The current build
is below; its selector scan lists these two as required ABSENCES.

Three more selectors joined that absence list on 2026-09-07 —
`getValidatorsWithKeysAt` `0x7cfba9f3`, `committeeSelectionEpoch` `0x8bd070e4`
and `getActiveValidatorsLengthAt` `0xd9b083ba`. A blob that still carries any of
them predates the single-selection change below.

Two more joined it later the same day — `settleEpochStipend` `0xa631344a` and
`settleEpochStipendFrom` `0x92d321ab`. A blob carrying either predates the change
that took the stipend out of this contract's balance, and a node built against it
expects a payment pass that no longer exists.

## This build — 2026-09-07 (second build of the day)

Rebuilt for one change: **the stipend never enters the staking contract**
(`.dpos-study/PLAN.md` 1.7).

**A — the epoch close prices the epoch and moves no money.** `close_epoch` used
to end in a fuel-capped self-call (`settleEpochStipendFrom`) that pulled the
epoch's whole pot from the BLEND reserve onto this contract and advanced a global
payment cursor; claims then paid out of that balance. The self-call, the cursor
`last_rewarded_epoch_p1` and the three claim gates that read it, `settle_up_to`,
`pay_epoch`, `MAX_SETTLE_CATCHUP`, `STIPEND_FUEL_CAP`, the `assigned_at_close_p1`
scalar, and BOTH `settleEpochStipend*` handlers are deleted. The close now asks
the token `min(balanceOf(reserve), allowance(reserve, staking))` through two
`static_call`s and forfeits the epoch — permanently, at zero — when that is below
the pot. An unreadable reserve scores zero rather than raising: the close is a
pre-execution system call, so a propagated error is an unrepairable chain halt.
Under-funding is a CLIFF, not a pro-rata payment, and a later top-up does not
revive an epoch that already closed at zero.

**B — the delegator claim splits in two.** `claimDelegatorFee` /
`claimDelegatorFeeAtEpoch` / `redelegateDelegatorFee` now mean REWARD ONLY, paid
by `transferFrom(reserve -> recipient)`. The matured undelegation principal moves
to a new `withdrawDelegatorPrincipal(address)` (`0xe75f359c`) paid by `transfer`
out of this contract's own balance, with a matching read
`getDelegatorPrincipal(address,address)` (`0xa789083d`). `getDelegatorFee`,
`getPendingDelegatorFee` and `calcAvailableForRedelegateAmount` therefore now
report the reward alone. The two cursors (`claimed_through_epoch`,
`undelegate_gap`) were already separate in storage; only the payment was merged.

**Consequence for the deployed `StakingPool` wrapper — NOT fixed here.**
`StakingPool.json` on this stand carries `claimDelegatorFee` (`0x426594b1`) three
times in its deployed bytecode, at two call sites. `claim(validator)` reads its
own BLEND balance before and after the call and treats the delta as
`principal + reward`; after this change that call returns the reward only and the
principal never arrives, so `claim` pays the staker out of whatever else the pool
holds. Worse: `withdrawDelegatorPrincipal` (`0xe75f359c`) appears NOWHERE under
`devnet/local-dpos-smoke/`, so the pool has no code path that can reach the new
handler at all — matured principal delegated through the pool is not merely
missing from `claim`, it is unwithdrawable by anyone until the pool is rebuilt.

The pool IS deployed and initialized at genesis (`genesis-bootstrap/src/bootstrap.rs`,
`deploy_to_canonical(..., STAKING_POOL_ADDR, ...)` and `"StakingPool.initialize"`),
and `initialize(address)` touches none of the changed ABI, so bring-up is
unaffected. What is dormant is only the runtime path: no harness case calls
`stake`/`unstake`/`claim`, and `STAKING_POOL_ADDR` is a retired constant in
`dpos_harness/core/topology.py`. A `StakingPool` rebuild is owed before anyone
exercises it.

- worktree HEAD: `c31c258f` (`refactor(staking)!: filter before the stake cut and
  drop the second selection`) — this is the 2026-09-07 A+B delta of the section
  below, now committed.
- worktree DIRTY at build time, deliberately. `git status --porcelain` — 14
  modified files, of which eleven are the contract:

      M contracts/staking/README.md          (the settlement cursor, the tolerant
                                              payment leg and the merged claim, all
                                              of which it still described)
      M contracts/staking/src/config.rs      (setBlendReserve still promised that an
                                              unapproved reserve merely DEFERS)
      M contracts/staking/src/consts.rs      (two ERC-20 read selectors in, the two
                                              settle selectors + MAX_SETTLE_CATCHUP +
                                              STIPEND_FUEL_CAP + two error codes out)
      M contracts/staking/src/events.rs      (StipendSkipped, StipendLegSkipped)
      M contracts/staking/src/initializer.rs (safe_transfer_from recipient)
      M contracts/staking/src/lib.rs         (two dispatch arms out, two in)
      M contracts/staking/src/liveness.rs    (settle_stipend_leg)
      M contracts/staking/src/staking.rs     (the claim split, the forfeit gate,
                                              pay_epoch/settle_up_to/both handlers)
      M contracts/staking/src/storage.rs     (last_rewarded_epoch_p1, assigned_at_close_p1)
      M contracts/staking/src/tests.rs
      M contracts/staking/src/util.rs        (safe_transfer_from takes a recipient;
                                              reserve_available added)

  `git diff --stat contracts/staking` — `11 files changed, 1244 insertions(+),
  1375 deletions(-)`. The other three are outside the crate and are part of the
  same change: `crates/testing/src/host.rs` (`static_call` was `unimplemented!()`
  and every close now makes one), `e2e/src/staking.rs`, `e2e/src/staking_cost.rs`.

- SHA-256 of every source file in `contracts/staking/src` as built (dirty
  content, not the committed content — `git show` will NOT reproduce the nine
  starred ones — `README.md` is not in `src/` and so is not hashed here):

      03060dbfb36f19c6b6058034bcd8465b570f80f507aa68344c2cc47ad960d886  src/config.rs      *
      2494ce8d0145eb6b9e70b5a75ccd0bd0de8928406bb77111e443085a6f4e973c  src/consensus.rs
      0ed1a9f2ebb1293c16e5aba28fc39c1a457e08268aa258f33ee6e2c76d2cd550  src/consts.rs      *
      82aefbe536c364085e9d345bb8a48b955ef6358d878433c08aa860b19d12539b  src/events.rs      *
      3f69dfe02d27be45e6b723e3f128b7049d74abe5c1b6dafac82b5af47d8b5576  src/evidence.rs
      634ab442666ba6ce61900c6d81a82a2cf266b9377ecc9751157c8564040c8bd8  src/initializer.rs *
      8ed61fc46cbf11c2aa3548fa116e0f2030257ae3c6503e61fc32a21cd6da2ed9  src/lib.rs         *
      8014b9c6f627bb0b5ead26cfb203e835b017e4899ff2b38657b34f941dca9c07  src/liveness.rs    *
      87fdd853b1c4d37cbc7421a07c8a6458afc486ff4300289b52015d9958fdf20f  src/math.rs
      7e489b52d6644442c9e4e1140cbe2f91a22dcc3718da00292b0f79efbcc3a491  src/staking.rs     *
      edd74a0494c58dd9ac02e6fd415f23e56a0154681446e149c6eb25a0e22622e9  src/storage.rs     *
      a4bb72fec738f7c8e77b5b7bcaee09ae5676dfb74a1ae48310394490894a3978  src/tests.rs       *
      de3751f4f574a205ae8d9aa6cf800d7098f040ac37415debc89cf106634ebe19  src/types.rs
      a76fa8763d1e7e9279b488295d53e02476990890806ff9b77e8ce77a02433327  src/util.rs        *

  Fourteen files, the same fourteen the sections below list. Ten starred. Reproduce with, from
  `contracts/staking`:

      find src -name '*.rs' | sort | xargs sha256sum

- `fluentbase_contracts_staking.wasm` — 411,835 bytes (was 409,972)
  `374a2aa6abfb690464d4508e3490744c5b56d0a89de8140a954a6b5d358b2ee3`
- `fluentbase_contracts_staking.rwasm` — 2,831,192 bytes (was 2,820,875)
  `ef59a62363f1ce1ede7f7b05abe90ff429279fde5fdde7d15c9a98972e6c8b61`
- Both digests differ from a build of this same change made forty minutes
  earlier at the SAME two sizes (`b4b77f8b…` / `046ad4de…`). The delta between
  the two source trees was comments and tests only. That is the
  non-reproducibility warned about below, observed again.
- Both GREW (+1,863 / +10,317) despite a net 411-line deletion: two new public
  handlers, two new outbound ERC-20 calldata builders and the two `static_call`
  sites cost more code than the settlement walk they replace, which was one
  scalar read and one transfer in a loop.
- **The build is not byte-reproducible from identical sources** — see the note
  under the 2026-09-07 section below; it still holds.
- **Gas, measured with `cargo test -p fluentbase-e2e staking_cost` after the
  change.** The worst epoch close fell from **4,014,257** to **3,954,277**
  (-59,980, 1.5%): the fuel-capped self-call and its four-epoch settlement walk
  left, less the two `static_call`s the close now makes. The accrual itself went
  from 3,214,963 to **3,211,372** (-3,591) — the two reads and their ABI
  encoding, measured rather than assumed. `commitEpochCommittee` did not move:
  1,814,093 + 17,100 per active-set entry, exhausting the 30M system-call budget
  at ~1,648 validators, exactly as below. `PATH_A_INTERCEPT_MAX` (4,640,000) and
  `PATH_A_ACCRUAL_MAX` (3,750,000) were left where they are — both still bind at
  ~17% over — and the new measurements recorded beside them.
  `STIPEND_LEG_MAX`, `STIPEND_CAP_GAS`, `MAX_SETTLE_CATCHUP` and the
  compile-time assert that tied the first two together were DELETED with the leg
  they priced, along with `path_a_stipend_leg_cost_at_max_catchup`.
- `cargo test` in `contracts/staking`: **153 passed, 0 failed** (159 before);
  **154 passed, 0 failed** with `--features devnet-views` (160 before). Twelve
  tests removed with the machinery they pinned, six added. `cargo test
  -p fluentbase-e2e`: **115 passed, 0 failed, 9 ignored**. `cargo test
  -p fluentbase-node` in the node tree: **59 passed, 0 failed**. `cargo clippy
  --all-targets` and `cargo fmt --check` clean in both workspaces.
- **Every new test was shown to go red.** Five mutations run and reverted:
  disable the reserve gate (kills 3), re-merge the reward and withdrawal cursors
  (kills 2), drop the `balance` half of `min(balance, allowance)` (kills 1), ask
  the reserve reads about this contract instead of the configured reserve (kills
  8), fold the matured deposit back into the reward view (kills 1).
- **Node-side decoder change, in the same commit.** `crates/node/src/evm.rs`
  decoded seven close-path events; `StipendSkipped` and `StipendLegSkipped` no
  longer exist, so their `sol!` declarations, their two `emit_close_observability`
  arms and the metrics `dpos_epoch_stipend_skipped_total` and
  `dpos_stipend_leg_skipped_total` are gone. The decode chain is CLOSED — an
  unmatched log falls through in silence — so this had to move with the contract.
  `EpochBlendRewardsCommitted` stays and now carries three more meanings of zero
  (reserve empty, approval missing, read failed) on top of the four it had; that
  indistinguishability is a deliberate decision, not an oversight.
- Selector scan on the `.rwasm`, run against this blob:

      present, 1 each:  recordProduction (0x1752910e) commitEpochCommittee (0xe505b249)
                        slashEquivocation (0xdc6fb3f2) producedAt (0x91c7d453)
                        getEpochCommitteeWithStakes (0xa4d160c1)
                        getEpochCommittee (0x80b562de) getEpochRewards (0x54c3e84b)
                        getDkgQual (0x2660899f) claimDelegatorFee (0x426594b1)
                        withdrawDelegatorPrincipal (0xe75f359c)
                        getDelegatorPrincipal (0xa789083d)
      absent, 0 each:   settleEpochStipend (0xa631344a)
                        settleEpochStipendFrom (0x92d321ab)
                        commitEpochBeaconKey (0x6ece9cb1) getEpochBeaconKey (0xc9adaf5c)
                        getValidatorsWithKeysAt (0x7cfba9f3)
                        committeeSelectionEpoch (0x8bd070e4)
                        getActiveValidatorsLengthAt (0xd9b083ba)

  **The two new OUTBOUND selectors are BIG-endian, not little.** A handler
  selector is compared against the incoming word and lands in the blob
  little-endian; calldata this contract BUILDS is a `to_be_bytes()` constant and
  lands big-endian. Scanning little-endian for them finds nothing and means
  nothing:

      big-endian, 1 each: balanceOf (0x70a08231) allowance (0xdd62ed3e)
                          transferFrom (0x23b872dd) transfer (0xa9059cbb)

- **NOT YET RUN ON THE STAND.** The smoke docker image has not been rebuilt
  against this blob and no devnet run has been made with it. Every golden
  snapshot is stale (`golden.py::_image_id`). Rebuild the image before trusting
  any of them.

### Previous build — 2026-09-07 (superseded)

Rebuilt for two changes (`.dpos-study/PLAN.md`).

**A — the committee carry-over is gone.** `commitEpochCommittee` reverts
`ERR_COMMITTEE_TOO_SMALL` again when the selection falls below
`MIN_COMMITTEE_LENGTH`, on every epoch and not only at `target == 0`.
`carry_committee_forward` and the `CommitteeCarriedOver` event are deleted. The
carry kept seats filled by validators the selection would no longer choose —
tombstoned ones included, still counted in the quorum denominator — and re-stamped
last epoch's weights, which aged without bound over a run of carries. The BFT
bound is accepted instead. This REOPENS what the 2026-09-04 build closed: a single
honest `undelegate` that takes the population under the floor now stops the chain,
deliberately.

**B — one selection algorithm, eligibility filtered before the stake cut.** The
committee is derived from `active_validators` with status Active, filtered to
members that are selection-visible and hold a consensus key active by the
selection epoch, THEN ranked by stake and cut to the cap. Filtering used to run
after the cut, so an ineligible validator occupied a seat and vacated it without
handing it on — twenty eligible validators could seat four. The separate selection
roster, the per-epoch cap checkpoints and three ABI points
(`getValidatorsWithKeysAt`, `committeeSelectionEpoch`,
`getActiveValidatorsLengthAt`) are deleted with it.

- worktree HEAD: `b22a8ed1` (`fix(staking)!: carry the committee forward instead
  of stopping the chain`)
- **HEAD moved since the 2026-09-04 section this replaces.** That section
  described a build from `bc42042a` DIRTY with three files; those are now
  committed as `b22a8ed1`, and this build's baseline is that commit.
- worktree DIRTY at build time, deliberately: the uncommitted delta is A + B and
  nothing else. `git status --porcelain contracts/staking` — 9 modified files,
  `9 files changed, 321 insertions(+), 808 deletions(-)`:

      M contracts/staking/src/config.rs      (cap checkpoints + getActiveValidatorsLengthAt)
      M contracts/staking/src/consensus.rs   (carry_committee_forward, selected_committee_at,
                                              getValidatorsWithKeysAt, committeeSelectionEpoch)
      M contracts/staking/src/consts.rs      (the three deleted selectors)
      M contracts/staking/src/events.rs      (CommitteeCarriedOver)
      M contracts/staking/src/lib.rs         (the three deleted dispatch arms)
      M contracts/staking/src/staking.rs     (selection roster, selected_validators_at,
                                              apply_production_exclusion's floor guard)
      M contracts/staking/src/storage.rs     (selection_roster, cap_checkpoints,
                                              CapCheckpointStorage, the `rostered` flag)
      M contracts/staking/src/tests.rs       (three tests removed, four rewritten)
      M contracts/staking/README.md          (the selection order and the cap paragraph, both
                                              of which stated the pre-change behaviour)

- SHA-256 of every source file in `contracts/staking/src` as built (dirty
  content, not the committed content — `git show` will NOT reproduce the eight
  starred ones):

      9bfe81e1ad21186d79d347da141b8506e2b44358b36501e8d263f27593a15ddd  src/config.rs      *
      2494ce8d0145eb6b9e70b5a75ccd0bd0de8928406bb77111e443085a6f4e973c  src/consensus.rs   *
      876d5e57ecd3be41d89e1e2c7a71f13e1217178470659fee8d2d55397f6cef1c  src/consts.rs      *
      19b44f8d3b01c1c0aba484cd32193bb98675a5398e8e512aefa01c80370b5584  src/events.rs      *
      3f69dfe02d27be45e6b723e3f128b7049d74abe5c1b6dafac82b5af47d8b5576  src/evidence.rs
      6e19613ec6ba6bc6ffe405b70ad998cc5ba4a1d05c42e851c11fc2f7f38e33f3  src/initializer.rs
      bed1267dde3cc7c5261590f2dbe2e7e93475ed98ed458562666b5e371eabb596  src/lib.rs         *
      2cb2284aa8385220c1e23a6c633f98b2954c4d2866f2626d206458fd6883f65e  src/liveness.rs
      87fdd853b1c4d37cbc7421a07c8a6458afc486ff4300289b52015d9958fdf20f  src/math.rs
      127c5dbc10665ef301cff4e5525aa0cca97357e44272f7d3f3ae90370d3d0377  src/staking.rs     *
      addcfc1f9141b04c002b34b5f1a9b5ec894d73ee10c4fb05e28a4714f6fd5771  src/storage.rs     *
      137c3f98800fb5c9e8bf6b3ca21458e0344c6f66a866337cc3f6b357d33c4752  src/tests.rs       *
      de3751f4f574a205ae8d9aa6cf800d7098f040ac37415debc89cf106634ebe19  src/types.rs
      6c2256b44b8c57d4ea0b34ca2adaf485591c2a0ba9f7d4f123ba6fe7b27f91a6  src/util.rs

  Fourteen files, the same fourteen the 2026-09-04 section listed. Reproduce
  with, from `contracts/staking`:

      find src -name '*.rs' | sort | xargs sha256sum

- `fluentbase_contracts_staking.wasm` — 409,972 bytes (was 416,781 on 2026-09-04)
  `c72e55796fdf4e30d07b0cc5567f515ba48bd92966409e0fc945cb18953adabe`
- `fluentbase_contracts_staking.rwasm` — 2,820,875 bytes (was 2,868,708)
  `a818fba45c31ebc158e3f3febca166cabbf327d6e379c58ab1d0f748013f785d`
- **The build is not byte-reproducible from identical sources.** An earlier build of this same
  change, differing only in doc comments, produced the SAME two sizes and DIFFERENT digests
  (`790aa305…42b5` / `765a1249…02e0`, and a third `dcb3f22f…dabe` / `a1a3a852…60ce`). So a digest mismatch against this record is evidence the
  sources moved, but a match is the only thing that proves they did not — do not read a mismatch
  as proof of a semantic change, and re-run the selector scan either way.
- Both SHRANK (-6,809 / -47,833): the carry-over branch, one event descriptor,
  three public handlers, the selection roster walk and the cap-checkpoint scan are
  all gone, and the new filter reuses the ranking that was already there.
- **Gas, measured with `cargo test -p fluentbase-e2e staking_cost` after the
  change.** The epoch close no longer scales with the validator set at all: its
  slope went from 4,600 gas per entry to **0**, because the exclusion guard now
  asks `eligible_population_at_least` — a scan that stops as soon as the floor is
  met — instead of counting the whole set. The commit went the other way, from
  12,800 to **17,100** per entry: moving the eligibility filter before the stake
  cut means the consensus-key read is paid for every candidate rather than only
  for the `cap` that were seated. That is the price of the ordering, not a slip,
  and the `COMMIT_SLOPE_MAX` / `COMMIT_ROSTER_FLOOR` thresholds in
  `e2e/src/staking_cost.rs` were re-pinned to the new measurements with the reason
  recorded beside each. At 17,100 the commit exhausts the 30M system-call budget
  at ~1,648 validators — thirty-two times the 51 the contract will accept.
- `cargo test` in `contracts/staking`: **159 passed, 0 failed** (162 before).
  Three tests were removed with the behaviour they pinned:
  `a_short_selection_carries_the_previous_committee_instead_of_stopping_the_chain`
  (the carry-over), `raising_the_cap_leaves_already_started_epochs_untouched` and
  `repeated_cap_changes_in_one_epoch_collapse_into_a_single_checkpoint` (the
  per-epoch cap history). The refusal below the floor stays covered by
  `a_refused_commit_writes_neither_committee_nor_cursor` and its sibling, which now
  reach it on the direct path.
- Selector scan on the `.rwasm`, run against this blob:

      present, 1 each:  recordProduction (0x1752910e) commitEpochCommittee (0xe505b249)
                        slashEquivocation (0xdc6fb3f2) producedAt (0x91c7d453)
                        getEpochCommitteeWithStakes (0xa4d160c1)
                        getEpochCommittee (0x80b562de) getEpochRewards (0x54c3e84b)
                        getDkgQual (0x2660899f)
      absent, 0 each:   commitEpochBeaconKey (0x6ece9cb1) getEpochBeaconKey (0xc9adaf5c)
                        getValidatorsWithKeysAt (0x7cfba9f3)
                        committeeSelectionEpoch (0x8bd070e4)
                        getActiveValidatorsLengthAt (0xd9b083ba)
                        resolveSigner getEpochCommitteeLength

  `CommitteeCarriedOver` is gone as an EVENT; events carry no selector to scan
  for, so its absence is pinned by the contract tests rather than by this scan.
- **NOT YET RUN ON THE STAND.** The smoke docker image has not been rebuilt
  against this blob and no devnet run has been made with it, so nothing here is
  live-verified beyond the build, the selector scan and the contract test suite.
  Every golden snapshot is stale (`golden.py::_image_id`). Rebuild the image
  before trusting any of them.

### Previous build — 2026-09-04 (superseded)

Carried the previous epoch's committee forward instead of reverting below
`MIN_COMMITTEE_LENGTH` — record pointer, length and re-stamped weight frame,
`dkgQual[target] = false`, and a `CommitteeCarriedOver(uint64 indexed epoch,
uint32 eligible, uint32 members)` event. Built from `bc42042a` dirty with three
files. `fluentbase_contracts_staking.wasm` 416,781 bytes
`c350bffb97e667fbcc152903dd534cacac2fbf112d213d4defe6ec56d4c3d91b`;
`fluentbase_contracts_staking.rwasm` 2,868,708 bytes
`988955e5fea07cc65d543b74933e68a7451a1375ea0fac98d980c198660697f4`. 162 contract
tests. Its per-file source hashes are in git history for this file. Superseded by
the build above, which removes that behaviour entirely.

### Previous build — 2026-08-08 (superseded)

Same HEAD `bb7d231d`, dirty with the same 12 files at an earlier content
(`12 files changed, 856 insertions(+), 1147 deletions(-)`), no source hashes
recorded. `fluentbase_contracts_staking.wasm` 417,946 bytes;
`fluentbase_contracts_staking.rwasm` 2,904,493 bytes; no SHA-256 recorded. It
carries no `commitEpochBeaconKey` / `getEpochBeaconKey` — like the post-rollback
build, though it predates several other node-side changes and is NOT a substitute
for one.
