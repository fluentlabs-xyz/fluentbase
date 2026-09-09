# Staking rWasm artefact — provenance

**[REVISED 2026-09-09]** Built from THIS tree: the contract sources live in
`contracts/staking/` since the merge `f16fdd90`, and the sibling worktree
`/home/djadjka/Work/audit-482/pr482-study` is history. Every section below dated
2026-09-08 or earlier was built from that worktree and says so; from the
2026-09-09 section down, "worktree HEAD" means this repository's HEAD.

The two blobs are still VENDORED here by hand — nothing copies them — so they
must be rebuilt and re-copied whenever `contracts/staking/src` moves.

Build command (run in the repository root):

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

Eleven more joined it on 2026-09-08 (task 1.5):
`changeValidatorOwner` `0x0052c9e1`, `getPendingValidatorFee` `0xc6fb9065`,
`getPendingDelegatorFee` `0xc2fd58fc`, `claimDelegatorFeeAtEpoch` `0xfe38ebef`,
`calcAvailableForRedelegateAmount` `0x5ef9e8c6`, `getValidatorsWithKeys`
`0xd41c52eb`, and the five constant getters `MAX_ACTIVE_VALIDATORS` `0x5d887462`,
`MAX_BLEND_STIPEND_PER_EPOCH` `0x2bc2fec4`, `DEFAULT_MIN_VERDICT_DUE_BLOCKS`
`0x6fd3afb7`, `DEFAULT_EXCLUSION_BACKOFF_CAP` `0xd4c30c1a` and
`MAX_MIN_VERDICT_DUE_BLOCKS` `0x9b9a11ba`. A blob carrying any of them predates
the dead-surface removal.

## This build — 2026-09-09 (Э2.1 + Э2.2, the shared-declaration crates)

Rebuilt because the contract now takes its selectors and its protocol limits from
two crates it shares with the node (`.dpos-study/PLAN.md` Э2.1 / Э2.2), instead
of declaring them a second time beside the node's own copies.

**No ABI point moved, in either direction, and the selector scan below is
unchanged and re-run.** What changed is where each value is DECLARED:

| value | before | now |
|---|---|---|
| 13 selectors the node calls + `AlreadySlashedForEquivocation` | `derive_keccak256_id!("…")` here, a `sol!` block in `node/src/evm.rs`, `reader.rs`, `slasher_sink.rs`, `slasher/actor.rs` | ONE `sol!` in `crates/staking-abi`; `consts.rs` computes `u32::from_be_bytes(<…Call as SolCall>::SELECTOR)` off it, the node imports the types |
| `MIN_COMMITTEE_LENGTH`, `MAX_ACTIVE_VALIDATORS_LENGTH`→`MAX_COMMITTEE_SIZE`, `BALANCE_COMPACT_PRECISION`, `uint112` width, `MAX_COMMITTEE_LOOKAHEAD_EPOCHS`, the four BLS widths, `PROPOSAL_PAYLOAD_LENGTH`, `fault_tolerance`, the epoch formula, `SYSTEM_CALLER` | a literal here and a second literal under `crates/dpos` | ONE declaration in `crates/types/src/staking_protocol.rs`, `pub use`d by both |

A node reading this contract sees no new selector, no removed one and no changed
one. The blob nevertheless MOVED, and by more than noise: the wasm is **4,358
bytes SMALLER** and the rwasm **23,822 bytes smaller**. Nothing in the change was
meant to shrink it; the plausible cause is that `sig::<C>()` folds to the same
constant the macro produced while several duplicated constants and one duplicated
function collapsed, and `alloy-sol-types`' generated encode/decode paths are dead
code the linker drops. This is recorded as an observation, not as a verified
explanation — no one measured which of the two crates accounts for which bytes.

- repository HEAD at build time: `f16fdd90`, working tree DIRTY (the Э2 change
  itself, uncommitted at build time — the two commits it becomes are made after
  this file is written). The per-file digests below are the record of what was
  actually compiled.
- SHA-256 of every source file in `contracts/staking/src` as built. Reproduce
  with, from `contracts/staking`: `find src -name '*.rs' | sort | xargs sha256sum`

      a710d7e5c7cbd7cbe84e4594b0840ad5992e68a3d4b7a24812f22222d6a444f4  src/bls.rs
      c3c2c67825dd810e63af18610f05770441a3a5fea44537bc11afd483c72749a1  src/config.rs      *
      c172a418600caa630d5a7e748ff4fabe6834029bacd5b1f351785cb6d988ce40  src/consensus.rs   *
      bbacb280992663fe3d057daa1cb6f912b8307b5fbbdb85f78652fa31f6e4409f  src/consts.rs      *
      6f9c65956d1d88ce36d426c626aad4b067b566459eb800812e3215aefe7f8535  src/events.rs
      3f69dfe02d27be45e6b723e3f128b7049d74abe5c1b6dafac82b5af47d8b5576  src/evidence.rs
      5f587627e81d7f38e52cfd974bf6c234de84f93925dd174f946b974dfac0987b  src/initializer.rs
      d4165b840ea4b7337a2a474ce62e81ee03b1c4c76d7fe6181d390741544f9aca  src/lib.rs
      46e17a0600efaba7d52f0b57f68f1fe75985c3a5f38d47fe7d6a92fc0bdba1a8  src/liveness.rs
      d106df9221d023e14adbec93ad329685208556ea0d1eb3946b4fde6bcad9b90f  src/math.rs        *
      c4d2c1388334c90ad5049adad153c72090bfef62121635de91848a337083a5f1  src/staking.rs
      d9b100f92acb6670c2626352b3cee7e0765b69d85feb28481441d0132cd83f54  src/storage.rs
      b43c8e34e460f2119f26d98c3051253f27928d8c61798b3af645393d5c07a01d  src/tests.rs       *
      e865fb1d3d6dd9832fde934d1cf530acf63054134ca167b0b8f0930f4a76e6fa  src/types.rs
      a76fa8763d1e7e9279b488295d53e02476990890806ff9b77e8ce77a02433327  src/util.rs

  Five starred files are the whole contract-side delta. `events.rs` is NOT
  starred by this work — its digest moved because of an unrelated uncommitted
  doc edit already in the tree. `src/tests.rs` is `#[cfg(test)]` and enters no
  artefact, so its digest moved AFTER this build (a review pass added
  `the_view_returns_decode_under_the_node_s_declaration`) without invalidating
  the blob; the value above is the current one. The blob depends on three inputs
  OUTSIDE this directory that no digest here covers:
  `crates/types/src/staking_protocol.rs`, `crates/staking-abi/src/lib.rs`, and
  `contracts/staking/Cargo.toml` (which is where the dependency on the second of
  them is declared).
- `fluentbase_contracts_staking.wasm` — 401,778 bytes (was 406,136, −4,358)
  `4dd7d27a747e1db10629fb453d3e6bcc522c5b0adfd87c476f58945dcf49e8f1`
- `fluentbase_contracts_staking.rwasm` — 2,775,337 bytes (was 2,799,159, −23,822)
  `bae5c9c863a3560eb5f9cb4739d3fc2a77c53cd0822b6dc3367b429e0e8f4522`
- Built with `cargo clean -p fluentbase-contracts` first, per the trap recorded
  under the 2026-09-08 (first) section.

### Selector scan of this blob

Unchanged from the previous section — Э2 moves no selector — and re-run against
the new blob. Run from this directory; all three groups must hold.

    python3 - <<'EOF'
    import pathlib
    b = pathlib.Path("fluentbase_contracts_staking.rwasm").read_bytes()
    must_be_1 = {"recordProduction":0x1752910e, "commitEpochCommittee":0xe505b249,
                 "slashEquivocation":0xdc6fb3f2, "producedAt":0x91c7d453,
                 "initialize":0xfecaf0f1, "getValidators":0xb7ab4db5,
                 "isValidatorActive":0x42ad55ac, "getValidatorStatus":0xa310624f,
                 "getEpochRewards":0x54c3e84b, "getDelegatorFee":0x52b7bea2,
                 "claimDelegatorFee":0x426594b1, "claimValidatorFeeAtEpoch":0xadf2a79c,
                 "redelegateDelegatorFee":0x8ecb3fc9, "getRegistryWithKeys":0xd96cbd7b,
                 "getValidatorFee":0x457179fd,
                 # added 2026-09-09: the rest of what the node calls, so the scan
                 # covers every point `fluentbase-staking-abi` declares.
                 "nextEpochToCommit":0xc06a82de,
                 "getEpochCommitteeWithStakes":0xa4d160c1, "getDkgQual":0x2660899f,
                 "getEpochBlockInterval":0x346c90a8, "getDposActivationBlock":0xa2a50528,
                 "getActiveValidatorsLength":0x32cc6f08, "getUndelegatePeriod":0x5e7b72ad,
                 "slashEquivocationNotarize":0xe28d2f63,
                 "slashEquivocationFinalize":0xadd07a3e,
                 "slashEquivocationNullifyFinalize":0xa10827e9}
    must_be_0 = {"changeValidatorOwner":0x0052c9e1, "getPendingValidatorFee":0xc6fb9065,
                 "getPendingDelegatorFee":0xc2fd58fc, "claimDelegatorFeeAtEpoch":0xfe38ebef,
                 "calcAvailableForRedelegateAmount":0x5ef9e8c6,
                 "getValidatorsWithKeys":0xd41c52eb, "MAX_ACTIVE_VALIDATORS":0x5d887462,
                 "MAX_BLEND_STIPEND_PER_EPOCH":0x2bc2fec4,
                 "DEFAULT_MIN_VERDICT_DUE_BLOCKS":0x6fd3afb7,
                 "DEFAULT_EXCLUSION_BACKOFF_CAP":0xd4c30c1a,
                 "MAX_MIN_VERDICT_DUE_BLOCKS":0x9b9a11ba,
                 "getValidatorsWithKeysAt":0x7cfba9f3, "committeeSelectionEpoch":0x8bd070e4,
                 "getActiveValidatorsLengthAt":0xd9b083ba, "settleEpochStipend":0xa631344a,
                 "settleEpochStipendFrom":0x92d321ab, "getBlsVerifier":0xc6b904ad,
                 "setBlsVerifier":0x466ae541, "commitEpochBeaconKey":0x6ece9cb1,
                 "getEpochBeaconKey":0xc9adaf5c}
    for n,s in must_be_1.items(): assert b.count(s.to_bytes(4,"little"))==1, n
    for n,s in must_be_0.items(): assert b.count(s.to_bytes(4,"little"))==0, n
    print("selector scan OK")
    EOF

Run against the blob recorded above: **`selector scan OK`**. The ten selectors
added to the `must_be_1` group on 2026-09-09 each appear exactly once, which is
the check that `fluentbase-staking-abi` and this blob agree: the crate's own
`selectors_match_the_deployed_artefact_scan` pins the same hex from the other
side.

### Tests

- `cargo test` in `contracts/staking`: **175 passed, 0 failed**; **176** with
  `--features devnet-views`. The pre-Э2 baseline was 174 WITH the feature; the
  no-feature baseline was never measured, so read 175/176 as the measurement and
  not as a delta against a number nobody took. Two new tests, both two-sided and
  both firsts: `close_event_topics_match_the_shared_abi` compares this crate's
  `#[derive(Event)]` topic0s against `fluentbase-staking-abi`'s `sol!` ones, and
  `the_view_returns_decode_under_the_node_s_declaration` hands the REAL handler's
  output bytes to the node's `abi_decode_returns` — the return SHAPE is the one
  thing sharing a `sol!` does not close by itself, because the handler encodes a
  Rust tuple through its own codec. `cargo clippy --all-targets --features
  devnet-views` and `cargo fmt --check`: clean.
- Root workspace: `cargo check --workspace` clean;
  `cargo test -p fluentbase-e2e --release` **113 passed / 9 failed / 9 ignored**,
  the nine being `builtins::*` fuel-accounting failures that predate the merge and
  do not touch this contract. Every `staking*` e2e is green against THIS blob.

## Previous build — 2026-09-08 (third build of the day)

Rebuilt for one change: **the reward split takes the vintage the committee was
selected from, and the kick ladder retires after a clean run**
(`.dpos-study/PLAN.md` 1.9), committed as `50e87d33`.

**No ABI point moved, in either direction.** 1.9 adds no handler, removes none
and renames none; the selector scan below is the previous section's, run again,
and it holds. What moved is what four existing entry points ANSWER for the same
chain state:

| entry point | what changed |
|---|---|
| `claimValidatorFee(address)`, `claimValidatorFeeAtEpoch(address,uint64)` | revert `ValidatorTombstoned(address)` for a tombstoned validator instead of paying |
| `getValidatorFee(address)` | answers zero for one, so the view agrees with the claim |
| `getDelegatorFee(address,address)`, `claimDelegatorFee(address)` | epoch E's reward is divided by the stake held at E−2, not at E, and charged `min(rate[E−2], rate[E])` |

A node reading this contract sees no new selector and no removed one. A node
reading the NUMBERS sees different ones, which is why this is a rebuild and not
a no-op.

Also in this build, and the reason the blob matters beyond 1.9:
`e2e/src/staking_reserve.rs` is new, and it is the first proof on real rWasm that
an epoch close survives a BLEND token that refuses the reserve read. The claim
in `util.rs`'s `reserve_available` doc comment used to rest on two e2e tests
deleted along with the self-call they covered, and on a unit host in which
`static_call` is routed to `call`. It now rests on three closes against this
blob: a token that reverts, one that burns every unit of fuel it is handed, and
one that answers a truncated word. See **Tests** below for the fuel figure,
which is a finding rather than a guard.

- worktree HEAD at build time: `50e87d33` (`feat(staking)!: divide the stipend
  by the selection vintage and retire the kick ladder after a clean run`).
- worktree CLEAN at build time — the first section in this file that can say so.
  Every earlier blob here was built from a dirty tree, which is why the per-file
  digests below exist at all. They are still recorded, and they still are the
  check: this build is reproducible from a named commit, but the build itself is
  not byte-reproducible (see the note under the 2026-09-07 section).
- SHA-256 of every source file in `contracts/staking/src` as built. Reproduce
  with, from `contracts/staking`: `find src -name '*.rs' | sort | xargs sha256sum`

      a710d7e5c7cbd7cbe84e4594b0840ad5992e68a3d4b7a24812f22222d6a444f4  src/bls.rs
      bea2696e32a65f8e8362610195f71302c692c38269e7eaab19de00f25dbb0d51  src/config.rs
      5fa775eaf8f1ed80ece0b5c9cbe1d2dc9620022f363a8100e84a19c223bdd2b2  src/consensus.rs
      572c3c4327e0745b6ee2281d61e3ec823ae996b29481244a5a39b3d1f7817ffa  src/consts.rs      *
      d45fb01e4297ae6343f30fda113c95cabbc9bb6114593e8c0a98249c60e48905  src/events.rs
      3f69dfe02d27be45e6b723e3f128b7049d74abe5c1b6dafac82b5af47d8b5576  src/evidence.rs
      5f587627e81d7f38e52cfd974bf6c234de84f93925dd174f946b974dfac0987b  src/initializer.rs
      d4165b840ea4b7337a2a474ce62e81ee03b1c4c76d7fe6181d390741544f9aca  src/lib.rs
      46e17a0600efaba7d52f0b57f68f1fe75985c3a5f38d47fe7d6a92fc0bdba1a8  src/liveness.rs    *
      87fdd853b1c4d37cbc7421a07c8a6458afc486ff4300289b52015d9958fdf20f  src/math.rs
      c4d2c1388334c90ad5049adad153c72090bfef62121635de91848a337083a5f1  src/staking.rs     *
      d9b100f92acb6670c2626352b3cee7e0765b69d85feb28481441d0132cd83f54  src/storage.rs     *
      86fb3a916db7313d27fd617c41d9b8d2d16151f2693aaf57c9eb0d511b644d5e  src/tests.rs       *
      e865fb1d3d6dd9832fde934d1cf530acf63054134ca167b0b8f0930f4a76e6fa  src/types.rs
      a76fa8763d1e7e9279b488295d53e02476990890806ff9b77e8ce77a02433327  src/util.rs

  The five starred files are 1.9's whole delta. `util.rs` is NOT starred, and
  that is worth one line: two mutations were run through it during this work
  (see **Tests**) and reverted, and its digest coming back to the value the
  previous section recorded is what proves the revert was byte-exact.

- `fluentbase_contracts_staking.wasm` — 406,136 bytes (was 404,068, +2,068)
  `e9ddd524bee4ea28b8f2bedfe12a4fcc1af1e2f138ec1080fcaacbad5b8b0dd7`
- `fluentbase_contracts_staking.rwasm` — 2,799,159 bytes (was 2,783,279, +15,880)
  `9f8e0241f5f5412fb6f9763fe25caf66eb390d6b33046ba070f011e404cc0bc2`
- Built with `cargo clean -p fluentbase-contracts` first, per the trap recorded
  under the 2026-09-08 (first) section: without it the wasm the build links can
  be stale. The digests above moved, which is the check that trap demands.
- **Cross-check the earlier sections could not make.** This rwasm digest is
  byte-identical to the artefact the contract worktree's own `e2e` suite linked
  while `staking_reserve.rs` ran — hashed out of
  `target/release/build/fluentbase-genesis-*/out/`. So the e2e evidence below is
  evidence about THIS blob, not about a sibling of it.

### Selector scan of that blob

Run from this directory; all three groups must hold. Unchanged from the previous
section — 1.9 moves no selector — and re-run against the new blob.

    python3 - <<'EOF'
    import pathlib
    b = pathlib.Path("fluentbase_contracts_staking.rwasm").read_bytes()
    must_be_1 = {"recordProduction":0x1752910e, "commitEpochCommittee":0xe505b249,
                 "slashEquivocation":0xdc6fb3f2, "producedAt":0x91c7d453,
                 "initialize":0xfecaf0f1, "getValidators":0xb7ab4db5,
                 "isValidatorActive":0x42ad55ac, "getValidatorStatus":0xa310624f,
                 "getEpochRewards":0x54c3e84b, "getDelegatorFee":0x52b7bea2,
                 "claimDelegatorFee":0x426594b1, "claimValidatorFeeAtEpoch":0xadf2a79c,
                 "redelegateDelegatorFee":0x8ecb3fc9, "getRegistryWithKeys":0xd96cbd7b,
                 "getValidatorFee":0x457179fd}
    must_be_0 = {"changeValidatorOwner":0x0052c9e1, "getPendingValidatorFee":0xc6fb9065,
                 "getPendingDelegatorFee":0xc2fd58fc, "claimDelegatorFeeAtEpoch":0xfe38ebef,
                 "calcAvailableForRedelegateAmount":0x5ef9e8c6,
                 "getValidatorsWithKeys":0xd41c52eb, "MAX_ACTIVE_VALIDATORS":0x5d887462,
                 "MAX_BLEND_STIPEND_PER_EPOCH":0x2bc2fec4,
                 "DEFAULT_MIN_VERDICT_DUE_BLOCKS":0x6fd3afb7,
                 "DEFAULT_EXCLUSION_BACKOFF_CAP":0xd4c30c1a,
                 "MAX_MIN_VERDICT_DUE_BLOCKS":0x9b9a11ba,
                 "getValidatorsWithKeysAt":0x7cfba9f3, "committeeSelectionEpoch":0x8bd070e4,
                 "getActiveValidatorsLengthAt":0xd9b083ba, "settleEpochStipend":0xa631344a,
                 "settleEpochStipendFrom":0x92d321ab, "getBlsVerifier":0xc6b904ad,
                 "setBlsVerifier":0x466ae541, "commitEpochBeaconKey":0x6ece9cb1,
                 "getEpochBeaconKey":0xc9adaf5c}
    for n,s in must_be_1.items(): assert b.count(s.to_bytes(4,"little"))==1, n
    for n,s in must_be_0.items(): assert b.count(s.to_bytes(4,"little"))==0, n
    print("selector scan OK")
    EOF

Run against the blob recorded above: **`selector scan OK`**.

### Tests

- `cargo test` in `contracts/staking`: **168 passed, 0 failed**; **169** with
  `--features devnet-views`. `cargo clippy --all-targets --features devnet-views`
  and `cargo fmt --check`: clean.
- `cargo test -p fluentbase-testing`: **3 passed** — new, and the subject is the
  harness rather than the contract. `static_call` in `crates/testing/src/host.rs`
  now fails the test if a mock writes storage inside it. It still routes to the
  same handler as `call`, so this does NOT make it a static frame: a mock that
  mutates only its own captured Rust state is invisible to it and always will
  be. What it catches is a mock reaching back into the host, which is the one
  class the host can see. The three tests are the guard firing, the same mock
  being legitimate through a plain `call`, and a read inside a static call still
  reaching its mock.
- `cargo test -p fluentbase-e2e --release` in the contract worktree's ROOT
  workspace: **120 passed, 0 failed, 9 ignored** (116/0/9 before — the four new
  ones are `staking_reserve`).
- **The reserve-read proof, and the fuel figure that came out of it.** All three
  refusals leave `recordProduction` successful, `getEpochRewards(E) == 0`, one
  `EpochBlendRewardsCommitted{E, 0}` committed, and the next epoch payable once
  `setBlendReserve` points at an address the token answers for. Frame gas of the
  close: 72,585 with a reverting token, 72,597 with a truncated word, and
  **1,968,777,406** with the fuel burner under a 2,000,000,000 transaction limit
  — the burner takes essentially everything it is offered, because
  `erc20_scalar_read` passes `fuel: None`.

  Re-run under the budget a node actually gives a system call — 30,000,000
  (`revm-rwasm` `crates/handler/src/system_call.rs`) — the close **survives**, at
  29,579,516 frame gas, leaving 420,484. It survives at committee 5, 21 and 51
  alike, and the figure does not move with the committee, because the forfeit arm
  returns before the committee walk: the reserve read is the LAST leg of
  `close_epoch`, and almost nothing follows it. The margin is EVM's 63/64 call
  rule and nothing else — no cap was set, none is asserted, and no test pins the
  number. Recorded as a measurement, not as a guard.
- **Both halves of the rule were shown by mutation, and both mutations reverted.**
  The blob digest was checked to have MOVED for each, per the trap under the
  first 2026-09-08 section, and to have come back to
  `9f8e0241…` afterwards.

  | mutation in `erc20_scalar_read` | result |
  |---|---|
  | a failed read answers `U256::MAX` instead of `U256::ZERO` | all four tests red: the epoch accrues its full pot off a token that refused |
  | a failed read propagates `Err` instead of scoring zero | all four tests red: `recordProduction` itself fails, which on a node is the block failing |

- **The devnet, on this blob and a rebuilt image.** The smoke image was rebuilt
  before any of it (`339c9553b5a9` → `4d41871fd9a8`), which is what invalidates a
  golden snapshot — `.vendor-sha` does not fingerprint this blob, see the note
  near the top of this file. Note also that the image does NOT contain this blob:
  `genesis-bootstrap` reads `fluentbase_contracts_staking.rwasm` out of the
  `/contracts` bind mount at container start (`genesis-bootstrap/src/artifacts.rs`),
  and `production_path.py` installs the `.wasm` at runtime. What a rebuild buys is
  the golden invalidation, not the blob.

  | run | result |
  |---|---|
  | `make case-growth` | PASS, 8m11s. Committee 4→5→6 across two boundaries, finalized 135→419, `dpos_dkg_pinned_idx_out_of_range_total=0` on 6/6 |
  | `scripts/xp/floor_halt_case.py 4 exit` | PASS. `CommitteeTooSmall(3, 4)` on 4/4 nodes, chain stopped at block 96, cursor never moved |
  | `scripts/xp/floor_halt_case.py 4 byz` | PASS. Same, on 3/3 honest nodes after the equivocator was tombstoned |
  | `scripts/xp/floor_halt_case.py 5 exit` | Chain LIVED to block 410, all five up; committee[3] re-seated without the leaver, `dkgQual[3] = true`. The script reports FAIL because it is written to assert the halt — at N=5 there is nothing to halt |
  | `scripts/xp/floor_halt_case.py 5 byz` | Same: chain lived to 410, committee[3] re-seated without the equivocator, `dkgQual[3] = true` |
  | `make smoke-byzantine` (five-node stand) | PASS, 8m00s. Equivocator jailed, transport severed, committee[3] re-seated 4 of 5 with `dkgQual` true, safety sweep clean |
  | one live claim | `claimValidatorFee(v)` from an address that is neither the validator nor its owner: 2 BLEND moved reserve → owner, `getValidatorFee` 2e18 → 0, staking contract's own BLEND balance unchanged at 5e18, 220,396 gas |

  Two things came out of those runs that are facts about the code rather than
  about the runs. First, the floor halt is an EXECUTION-plane halt: the leader
  proposes the boundary order block, consensus agrees it, and every node then
  fails at `try_derive` → `derive.rs:604` → `executor fatal error … stage="finalize"`
  within 31 ms of each other, exiting cleanly. It is refused neither while the
  block is BUILT nor while it is VALIDATED, which is the question
  `floor_halt_case.py`'s own docstring left open. Second, the 1.9 commission rule
  was observed on a chain: a 10% rate set in epoch 17 (stamped `changed_at = 18`)
  first reached money at reward epoch 20 and paid 0.25 BLEND per epoch on a
  2.5-BLEND seat share — `min(rate[E−2], rate[E])`, exactly.

### Previous build — 2026-09-08 (second build of the day, superseded)

Rebuilt for one change: **eleven ABI points with no consumer are deleted**
(`.dpos-study/PLAN.md` 1.5, finding KB-6).

Deleted, selector and handler and dispatch arm: `changeValidatorOwner`
(`0x0052c9e1` — it validated the caller and then always reverted
`ValidatorOwnerImmutable()`; the error id and `TwoAddressesCommand` go with it),
`getPendingValidatorFee` (`0xc6fb9065`), `getPendingDelegatorFee`
(`0xc2fd58fc`), `claimDelegatorFeeAtEpoch` (`0xfe38ebef`),
`calcAvailableForRedelegateAmount` (`0x5ef9e8c6`), `getValidatorsWithKeys`
(`0xd41c52eb`), and the five constant getters `MAX_ACTIVE_VALIDATORS`
(`0x5d887462`), `MAX_BLEND_STIPEND_PER_EPOCH` (`0x2bc2fec4`),
`DEFAULT_MIN_VERDICT_DUE_BLOCKS` (`0x6fd3afb7`),
`DEFAULT_EXCLUSION_BACKOFF_CAP` (`0xd4c30c1a`) and `MAX_MIN_VERDICT_DUE_BLOCKS`
(`0x9b9a11ba`).

**No ABI point outside that list moved.** `initialize` is untouched — same
sixteen arguments, same `0xfecaf0f1` — so `genesis-bootstrap/src/bootstrap.rs`
and `dpos_harness/stack/production_path.py` need no change this time, and
`min_undelegate_blocks` stays. The five Rust constants behind the deleted getters
are unchanged and still bound their setters; only the read points are gone.

**Kept, against the same candidate list, each with a named consumer.** These four
were on the "no consumer" list and are NOT deleted:

| kept | consumer |
|---|---|
| `getValidators()` `0xb7ab4db5` | the DEPLOYED `FluentGovernance` runtime: `PUSH4 b7ab4db5` at offset 10400 of `FluentGovernance.json::deployedBytecode`, inside `_votingSupply`, on `_stakingContract` — which `bootstrap.rs::deploy_governance` constructs as `(STAKING_ADDR, STAKING_ADDR)` |
| `isValidatorActive(address)` `0x42ad55ac` | two call sites in `FluentGovernance.json`'s own `ast` (`onlyValidatorOwner`, guarding `propose`/`proposeWithCustomVotingPeriod`, and `_validatorOwnerVotingPowerAt`) — see the caveat below |
| `getValidatorStatus(address)` `0xa310624f` | the python stand: `dpos_harness/core/nodes.py:567` `VALIDATOR_STATUS_SIG` |
| `getEpochRewards(uint64)` `0x54c3e84b` | the CONTRACT worktree's own root workspace: `e2e/src/staking_cost.rs:315` declares it, `:548` calls it, `:797` asserts on it |

**Caveat that kept `isValidatorActive`, stated because it is unresolved.**
`FluentGovernance.json` is internally inconsistent: its `ast` describes external
calls its `bytecode` does not make. `isValidatorActive` `0x42ad55ac`,
`getValidatorByOwner` `0x30108c22` and the error `OnlyValidatorOwner()`
`0xce66db66` are named in the ABI/AST and absent from both `bytecode` and
`deployedBytecode` — checked twice, by a raw hex substring scan (alignment-
independent, so it also covers a selector sitting inside a PUSH32 immediate) and
by an opcode-aligned PUSH4 walk. `StakingPool.json` shows the same pattern
(`currentEpoch`, `getDelegatorFee`, `undelegate` in its AST, absent from its
bytecode). The mismatch is ASYMMETRIC: from the same AST, `getValidators` and
`getValidatorDelegatedStakeAt` ARE in the governance bytecode.

The blob is older than the **AST**, not than the ABI — measured, because the
obvious explanation had to be ruled out: all 53 of `FluentGovernance`'s
`methodIdentifiers` are present in its `deployedBytecode`, none missing
(`StakingPool` has exactly one absent, `getShares(address,address)`). Editing an
INTERNAL function does not move the ABI, and every one of these call sites is in
an internal function or modifier — `onlyValidatorOwner`,
`_validatorOwnerVotingPowerAt`, `_countVote` — which fits. That last step is
inference: neither `FluentGovernance.sol` nor `StakingPool.sol` exists in either
tree, only the compiled artefact, so the cause cannot be proven here.

What this means for `isValidatorActive`: its keep does NOT rest on a consumer in
running code — there is none today. It rests on two call sites in the source that
artefact was compiled from. A regenerated `FluentGovernance` WOULD call it, and
deleting it would then break `propose()` and `proposeWithCustomVotingPeriod()`
for every validator owner, silently. It stays until someone regenerates these
artefacts and re-runs the scan. The direction of the mismatch is also why it does
not weaken the eleven deletions: it produces AST calls absent from bytecode,
never bytecode calls absent from source, and the bytecode was scanned directly.

- worktree HEAD at build time: `f70ceffe` (`refactor(staking)!: verify BLS in the
  module instead of a settable verifier`) — the 2026-09-08 BLS change below,
  which is why the HEAD had moved from `100c02c4`.
- worktree DIRTY at build time, deliberately: the 8-file delta below
  (`README.md`, `config.rs`, `consensus.rs`, `consts.rs`, `lib.rs`, `staking.rs`,
  `tests.rs`, `types.rs`; nothing outside `contracts/staking`).
  **That delta is now committed as `2fa46f1b`** (`refactor(staking)!: drop eleven
  ABI points with no consumer`), so unlike every earlier section here these blobs
  ARE reproducible from a named commit — check out `2fa46f1b` in that worktree
  and the source digests below are what you get. The build itself is still not
  byte-reproducible (see the note under the previous section); the digests remain
  the check.
- SHA-256 of every source file in `contracts/staking/src` as built. Reproduce
  with, from `contracts/staking`: `find src -name '*.rs' | sort | xargs sha256sum`

      a710d7e5c7cbd7cbe84e4594b0840ad5992e68a3d4b7a24812f22222d6a444f4  src/bls.rs
      bea2696e32a65f8e8362610195f71302c692c38269e7eaab19de00f25dbb0d51  src/config.rs      *
      5fa775eaf8f1ed80ece0b5c9cbe1d2dc9620022f363a8100e84a19c223bdd2b2  src/consensus.rs   *
      4d50bd8fb9fb7fb3716eaf3942cd659e0292270ce2d5d2ebc1eeb4b67d0e34f2  src/consts.rs      *
      d45fb01e4297ae6343f30fda113c95cabbc9bb6114593e8c0a98249c60e48905  src/events.rs
      3f69dfe02d27be45e6b723e3f128b7049d74abe5c1b6dafac82b5af47d8b5576  src/evidence.rs
      5f587627e81d7f38e52cfd974bf6c234de84f93925dd174f946b974dfac0987b  src/initializer.rs
      d4165b840ea4b7337a2a474ce62e81ee03b1c4c76d7fe6181d390741544f9aca  src/lib.rs         *
      8014b9c6f627bb0b5ead26cfb203e835b017e4899ff2b38657b34f941dca9c07  src/liveness.rs
      87fdd853b1c4d37cbc7421a07c8a6458afc486ff4300289b52015d9958fdf20f  src/math.rs
      f964364b554523e5c5df250620c2469b0477a1ed8ddef8ce3b6defbc8cb6d85d  src/staking.rs     *
      a4533236f45682cdd955dc7f58ff34e2df8f5c14bf05ff7e71912387ba8d9b86  src/storage.rs
      e66d0c5a58df245590b6e39c0b2b723f575dd7192a80375eabc64ffc3f314806  src/tests.rs       *
      e865fb1d3d6dd9832fde934d1cf530acf63054134ca167b0b8f0930f4a76e6fa  src/types.rs       *
      a76fa8763d1e7e9279b488295d53e02476990890806ff9b77e8ce77a02433327  src/util.rs

- `fluentbase_contracts_staking.wasm` — 404,068 bytes (was 412,702, −8,634)
  `6950255e0cd4abea52e70b43d796ecfd04ae08f771ca38d79af2e56facf2aa49`
- `fluentbase_contracts_staking.rwasm` — 2,783,279 bytes (was 2,834,075, −50,796)
  `5466af2a12bd5dde6bff1951b4e2f846e4a03d5c38dab44593b9c05742d22f9c`
- Built with `cargo clean -p fluentbase-contracts` first, per the trap recorded
  under the previous section: without it the wasm the build links can be stale.
  The digests above moved, which is the check that trap demands.
- `src/tests.rs` and `README.md` moved AFTER that build (the coverage repair
  described under **Tests**). Both are outside the artefact: `tests.rs` is
  `#[cfg(test)]` and `README.md` is not code. Verified rather than assumed — the
  blobs were rebuilt from the current sources and came out byte-identical to the
  two digests above.

### Selector scan of that blob

Run from this directory; all three groups must hold.

    python3 - <<'EOF'
    import pathlib
    b = pathlib.Path("fluentbase_contracts_staking.rwasm").read_bytes()
    must_be_1 = {"recordProduction":0x1752910e, "commitEpochCommittee":0xe505b249,
                 "slashEquivocation":0xdc6fb3f2, "producedAt":0x91c7d453,
                 "initialize":0xfecaf0f1, "getValidators":0xb7ab4db5,
                 "isValidatorActive":0x42ad55ac, "getValidatorStatus":0xa310624f,
                 "getEpochRewards":0x54c3e84b, "getDelegatorFee":0x52b7bea2,
                 "claimDelegatorFee":0x426594b1, "claimValidatorFeeAtEpoch":0xadf2a79c,
                 "redelegateDelegatorFee":0x8ecb3fc9, "getRegistryWithKeys":0xd96cbd7b,
                 "getValidatorFee":0x457179fd}
    must_be_0 = {"changeValidatorOwner":0x0052c9e1, "getPendingValidatorFee":0xc6fb9065,
                 "getPendingDelegatorFee":0xc2fd58fc, "claimDelegatorFeeAtEpoch":0xfe38ebef,
                 "calcAvailableForRedelegateAmount":0x5ef9e8c6,
                 "getValidatorsWithKeys":0xd41c52eb, "MAX_ACTIVE_VALIDATORS":0x5d887462,
                 "MAX_BLEND_STIPEND_PER_EPOCH":0x2bc2fec4,
                 "DEFAULT_MIN_VERDICT_DUE_BLOCKS":0x6fd3afb7,
                 "DEFAULT_EXCLUSION_BACKOFF_CAP":0xd4c30c1a,
                 "MAX_MIN_VERDICT_DUE_BLOCKS":0x9b9a11ba,
                 "getValidatorsWithKeysAt":0x7cfba9f3, "committeeSelectionEpoch":0x8bd070e4,
                 "getActiveValidatorsLengthAt":0xd9b083ba, "settleEpochStipend":0xa631344a,
                 "settleEpochStipendFrom":0x92d321ab, "getBlsVerifier":0xc6b904ad,
                 "setBlsVerifier":0x466ae541, "commitEpochBeaconKey":0x6ece9cb1,
                 "getEpochBeaconKey":0xc9adaf5c}
    for n,s in must_be_1.items(): assert b.count(s.to_bytes(4,"little"))==1, n
    for n,s in must_be_0.items(): assert b.count(s.to_bytes(4,"little"))==0, n
    print("selector scan OK")
    EOF

Run against the blob recorded above: **`selector scan OK`**.

### Tests

- `cargo test` in `contracts/staking`: **160 passed, 0 failed** (162 before);
  **161** with `--features devnet-views` (163 before). Two tests removed with the
  handlers they covered, named:
  `validator_owner_is_immutable_and_cannot_detach_self_stake` (it existed only to
  assert `changeValidatorOwner`'s revert) and
  `embedded_chain_config_exposes_solidity_public_constants` (it existed only to
  read `MAX_ACTIVE_VALIDATORS()` and `MAX_BLEND_STIPEND_PER_EPOCH()`). Five more
  tests lost assertions but kept their subject:
  `get_consensus_keys_matches_dynamic_struct_return_vectors`,
  `reward_views_split_blend_between_owner_and_delegators`,
  `the_delegator_views_report_the_reward_and_the_deposit_apart`,
  `derived_selectors_match_independent_hex_pins` and
  `production_liveness_ships_disabled_on_a_fresh_chain`. Each was shown red again
  by a mutation, and every mutation reverted:

  | mutation | kills |
  |---|---|
  | `get_consensus_keys` returns `ConsensusKeys::default()` | `get_consensus_keys_matches_dynamic_struct_return_vectors` |
  | `write_validators_with_keys` returns two empty vectors | `get_consensus_keys_matches_dynamic_struct_return_vectors` |
  | `get_delegator_fee` returns the reward + 1 | `reward_views_split_blend_between_owner_and_delegators`, `the_delegator_views_report_the_reward_and_the_deposit_apart`, `future_delegation_and_noop_commission_do_not_bypass_warmup` |
  | the pinned signature string `currentEpoch()` drifts to `currentEpochX()` | `derived_selectors_match_independent_hex_pins` |
  | `get_min_verdict_due_blocks` returns 0 | `production_liveness_setters_enforce_their_bounds`, `production_liveness_ships_disabled_on_a_fresh_chain` |

  **A coverage hole this change opened, and closed.** The deleted
  `getValidatorsWithKeys` block in
  `get_consensus_keys_matches_dynamic_struct_return_vectors` was the ONLY
  assertion anywhere over `write_validators_with_keys`, the shared encoder behind
  it and `getRegistryWithKeys` (`0xd96cbd7b`) — and `SIG_GET_REGISTRY_WITH_KEYS`
  appears nowhere in `tests.rs`, at this HEAD or before it. Removing the block
  therefore left a live production handler with zero coverage: measured, a
  `write_validators_with_keys` that answers two empty vectors passed all 161
  tests. The assertion was moved onto `SIG_GET_REGISTRY_WITH_KEYS` (same return
  shape), and the second row of the table above is that same mutation failing
  now. Found by an adversarial review pass, not by the author.
- `cargo test -p fluentbase-e2e` in the contract worktree's ROOT workspace:
  **116 passed, 0 failed, 9 ignored** — this is the workspace an earlier
  "no consumers" search missed, and it is where `getEpochRewards`'s consumer
  lives.
- `cargo clippy -p fluentbase-contracts-staking --all-targets --features
  devnet-views` and `cargo fmt --check`: clean.
- In THIS tree: `cargo test -p fluentbase-node` **59 passed, 0 failed**.
  `cargo test -p fluentbase-e2e` **100 passed, 9 failed** — all nine are
  `builtins::*` hardcoded gas pins (e.g. `builtins.rs:87`, 23,607 against an
  expected 23,095), pre-existing at this tree's HEAD and untouched by this change
  (no `.rs` file in this tree was modified).
- **NOT run: the devnet.** This blob has not been on a live stand. The previous
  section's `make case-growth` result does not carry over.

### Previous build — 2026-09-08 (first build of the day, superseded)

Rebuilt for one change: **the BLS verifier is inside the module**
(`.dpos-study/BLS_VERIFY_PORT_SPEC.md`, task 1.10).

The module used to reach a verifier contract through an address in its own
storage, moved by a governance setter (`setBlsVerifier`). A substituted verifier
accepts a forged proof of possession, and a forged PoP forges a whole committee's
quorum (`pk' = pk_x - Σ pk_i`). That address is gone. `contracts/staking/src/bls.rs`
is a port of `solidity-contracts@f641789f:contracts/libraries/BLS12381Verifier.sol`
and calls the EIP-2537 precompiles itself — `0x02` SHA-256, `0x05` MODEXP, `0x0b`
G1ADD, `0x0f` PAIRING, `0x10` MAP_FP_TO_G1 — at addresses the fork fixes and no
setter can move. The arithmetic did not move; only the byte assembly and the calls
did.

**ABI break.** `initialize` loses its `address blsVerifier` (argument 15) and goes
from seventeen arguments to sixteen: `0xdfa8efb0` -> `0xfecaf0f1` (both from
`cast sig`). `getBlsVerifier` (`0xc6b904ad`) and `setBlsVerifier` (`0x466ae541`)
are deleted, with the `BlsVerifierChanged` event, the
`BlsVerifierNotConfigured()` error and the `bls_verifier` slot in
`ChainConfigStorage` — which shifts every slot below it. Nothing outside the crate
reads that layout and nothing is deployed.

**Two places outside the contract broke SILENTLY and are fixed in the same
change**: `genesis-bootstrap/src/bootstrap.rs` kept its own `sol!` copy of the
interface (it would have gone on compiling and sending `0xdfa8efb0`), and
`dpos_harness/stack/production_path.py` builds the calldata from a hardcoded
signature string. Both now carry sixteen arguments. `BLS12381Verifier.json`, its
`Makefile` regen line, the `0x…5208` predeploy and the `forge create` of it are
deleted; the address is left vacant rather than reassigned.

- worktree HEAD: `100c02c4` (`feat(staking)!: draw the stipend from the reserve
  instead of the contract balance`), unchanged since the previous section.
- worktree DIRTY at build time, deliberately. The contract delta is 10 files —
  `bls.rs` NEW, and `config.rs`, `consensus.rs`, `consts.rs`, `events.rs`,
  `initializer.rs`, `lib.rs`, `storage.rs`, `types.rs`, `tests.rs` modified. Outside the crate and
  part of the same change: `e2e/src/staking.rs`, `e2e/src/staking_cost.rs`,
  `e2e/src/staking_bls.rs` (new), `e2e/src/bls_vectors.rs` +
  `e2e/src/bls_pop_vectors.bin` (new), `contracts/Cargo.toml` (excludes a
  hook-created `.claude` directory that breaks the workspace glob).
- SHA-256 of every source file in `contracts/staking/src` as built. Reproduce
  with, from `contracts/staking`: `find src -name '*.rs' | sort | xargs sha256sum`

      a710d7e5c7cbd7cbe84e4594b0840ad5992e68a3d4b7a24812f22222d6a444f4  src/bls.rs         * NEW
      6e4b83186c70ef0682f5e29b6666bc96a999081a0cd5fd30041981d18fed8b5e  src/config.rs      *
      617efa37c505910402940ddfd4ecacb9f3578ae9d21804c2ffd852900c137f09  src/consensus.rs   *
      2a157fa3ba7829dcb171f52f794ecfd4309e3d73dd3b7c690dda5dfe83dc396e  src/consts.rs      *
      d45fb01e4297ae6343f30fda113c95cabbc9bb6114593e8c0a98249c60e48905  src/events.rs      *
      3f69dfe02d27be45e6b723e3f128b7049d74abe5c1b6dafac82b5af47d8b5576  src/evidence.rs
      5f587627e81d7f38e52cfd974bf6c234de84f93925dd174f946b974dfac0987b  src/initializer.rs *
      4218de7942e6b247c76b15c1ffc84e27caf49126121f8f878ecee7bffde15b4c  src/lib.rs         *
      8014b9c6f627bb0b5ead26cfb203e835b017e4899ff2b38657b34f941dca9c07  src/liveness.rs
      87fdd853b1c4d37cbc7421a07c8a6458afc486ff4300289b52015d9958fdf20f  src/math.rs
      7e489b52d6644442c9e4e1140cbe2f91a22dcc3718da00292b0f79efbcc3a491  src/staking.rs
      a4533236f45682cdd955dc7f58ff34e2df8f5c14bf05ff7e71912387ba8d9b86  src/storage.rs     *
      2321f7d673f28190b260d80f87f96fdedd00d8430d86d380b20cb3f48d7c3924  src/tests.rs       *
      4c5c27e9faccfb65ae1eaf325ec00bd2515a8b6cafd1724a8d0169369c25c115  src/types.rs       *
      a76fa8763d1e7e9279b488295d53e02476990890806ff9b77e8ce77a02433327  src/util.rs

  These fifteen are the sources as they stand, re-taken after the review pass
  below. `consensus.rs` and `initializer.rs` moved after the first build (a
  comment corrected in each) and `tests.rs` moved twice (tests added); the wasm
  and rwasm were REBUILT from exactly these sources afterwards and came out
  byte-identical to the blobs recorded above, which are the ones the devnet ran.
  An earlier revision of this section pinned a stale `consensus.rs` digest — the
  adversarial review below caught it, and it is worth saying why it happened: a
  workspace-wide `cargo fmt` ran between the hashing and the recording.
- `fluentbase_contracts_staking.wasm` — 412,702 bytes (was 411,835)
  `238cd91e13833db2a0b7a6c1fce25d1243b8cda888bd03a92915c787b14c747a`
- `fluentbase_contracts_staking.rwasm` — 2,834,075 bytes (was 2,831,192)
  `96cbf32fb61b26d72dc054cb6d53ba467c961ddc66611fbc4b4bb966b5c3fcf4`
- Both GREW (+867 / +2,883) despite five deleted handlers and a deleted storage
  slot: the hash-to-curve pipeline, the two compressions and the five precompile
  call sites cost more code than the seven ABI-encoded external calls they replace.
- **The build is not byte-reproducible from identical sources** — the note under
  the 2026-09-07 section still holds. A digest mismatch is evidence the sources
  moved; a match is the only thing that proves they did not.

### Gas, measured

Two independent measurements, both real, neither an estimate.

**1. The two contract paths, before and after, on real rWasm.** Same harness
(`e2e`, `EvmTestingContext` with the full genesis), same arkworks EIP-2537
predeploys, real `blst`-made signatures on both sides. The "before" run is a git
worktree of `100c02c4` with the REAL `BLS12381Verifier` deployed from
`BLS12381Verifier.json` — not the mock the e2e fixtures used to install, which
would have measured nothing. Frame gas, EVM transaction intrinsic removed:

| path | before (external verifier) | after (inline) | delta |
|---|---|---|---|
| `registerValidator` (1 × compressG2 + 1 × verify) | 606,273 | **491,418** | −114,855 (−18.9 %) |
| `slashEquivocationNotarize` (1 × compressG2 + 2 × compressG1 + 2 × verify) | 605,524 | **372,655** | −232,869 (−38.5 %) |

Both numbers come from `cargo test -p fluentbase-e2e --release both_bls_paths -- --nocapture`.
The saving is the seven `CALL`s and their ABI encode/decode, not the cryptography:
the pairings cost the same either way.

**2. The retired verifier's own cost, reproduced on anvil.** `anvil 1.6.0-v1.7.0
--hardfork prague`, the artefact deployed, `gasUsed` from the receipt. This
reproduces `BLS_VERIFY_PORT_SPEC.md` §7 rather than copying it:

| call | tx gasUsed | execution | spec §7 execution |
|---|---|---|---|
| `verify` (PoP, valid) | 195,861 | **165,789** | 165,789 |
| `compressG2Unchecked` (256 B) | 85,229 | 60,581 | 60,812 |
| `compressG1Unchecked` (128 B) | 57,163 | 34,155 | 34,143 |
| `unionUnique` (23+96 B) | 27,140 | 3,684 | 3,738 |
| bare `PAIRING` precompile, 2 pairs | 133,860 | **102,900** | 102,900 |

`verify` and the bare pairing reproduce exactly; the two compressions differ by
~230 gas because a different vector was used and its y-sign branch differs.

**A trap worth recording.** `cast send` without `--gas-limit` takes its limit from
`eth_estimateGas`, and this anvil under-estimates an EIP-2537-heavy call by half
(99,305 against 195,861) — and then REPORTS THE ESTIMATE as `gasUsed`, with
`status = 0x1`. The first run of this measurement produced 99,305 and looked
perfectly ordinary. Every figure above was taken with an explicit
`--gas-limit 3000000`.

### Tests

- `cargo test` in `contracts/staking`: **162 passed, 0 failed** (153 before);
  **163** with `--features devnet-views` (154 before). One test removed with the
  branch it covered (`registration_rejects_non_96_byte_compressed_key_...` — the
  compression returns a fixed-width array now, so the branch is gone), ten added.
- `cargo test -p fluentbase-e2e --release`: staking suites green.
- `cargo test -p fluentbase-genesis-bootstrap`: **8 passed**, including two new
  ones — the stored identity equals `blst`'s compression of the same key, and a
  proof of possession bound to another chain is refused.
- `make harness-test`: **2082 passed, 10 skipped**.
- `cargo clippy --all-targets` and `cargo fmt --check` clean.
- **Every new or rewritten test was shown red by a mutation, and the mutation
  reverted.** Fifteen mutations, across all four suites:

  | mutation | kills |
  |---|---|
  | `verify` ignores the pairing verdict | the forged-PoP unit test, the slash-signature test, the wrong-entry-point test, the e2e wrong-key PoP, the wrong-chain PoP in `bootstrap_smoke` |
  | G2 compression keeps the EIP-2537 half order | the compression test, the two stored-key tests, the e2e blst-compression check, `bootstrap_smoke`'s |
  | the y-sign compare becomes non-strict | the strictness test |
  | the y-sign bit is never set (G2) | six unit tests |
  | the y-sign bit is never set (G1) | the G1 sign test |
  | G1 reads `x` from the `y` half | the G1 sign test, the domain-separator test |
  | infinity is compressed instead of refused | the infinity/width test |
  | the namespace guard is off by one | the namespace test |
  | the DST guard is off by one | the DST test |
  | the precompile output width is unchecked | the wrong-width test |
  | a failed pairing reverts instead of answering false | the refused-pairing test |
  | every evidence kind hashes under one domain | the domain-separator test |
  | the evidence path signs under the PoP DST | `both_bls_paths` |
  | the evidence namespace loses its kind suffix | `both_bls_paths` |
  | the harness `initialize` signature keeps seventeen arguments | the python bring-up test |

- **What the mutation pass found, and what it cost to find.** The first attempt
  at the e2e mutations was WRONG twice over, and both traps are worth recording.

  1. Editing `contracts/staking/src/**` does not reliably rebuild the wasm that
     `e2e` links: cargo's `rerun-if-changed` on the `contracts` directory did not
     fire, and one run compared a mutated artefact against a stale one and called
     it a baseline. A mutation pass over this boundary MUST check that the wasm
     digest actually moved. `cargo clean -p fluentbase-contracts` before the
     rebuild is what makes it move.
  2. The first e2e forgery flipped one bit of a proof of possession. That puts
     the point off the curve, and the EIP-2537 pairing precompile REFUSES such
     input rather than answering "does not verify" — so the rejection came from
     the precompile-status check and the verdict branch was never reached. The
     test survived `verify` returning `true` unconditionally and looked fine.
     The test now also feeds ANOTHER validator's proof of possession: a
     well-formed point in the right subgroup that proves possession of a
     different key. That one reaches the verdict, and the mutation kills it.

### The unit harness stubs the precompiles, and says so

`contracts/staking/src/tests.rs` answers `0x02`/`0x05`/`0x0b`/`0x0f`/`0x10` by
address. SHA-256 is honest — `crypto_sha256`, the same implementation `0x02` runs
— so the namespace a test reads back is the one the contract really hashed.
MODEXP, MAP_FP_TO_G1 and G1ADD return deterministic values of the right width.
PAIRING is a policy, not a pairing: it answers "the equation holds" unless a test
says otherwise. That switch is the point — the retired external-verifier stub
answered `true` unconditionally, so `InvalidProofOfPossession` and the verify half
of `EquivocationSignatureInvalid` were unreachable from any test in the file. It
decides nothing about BLS12-381; that lives in `e2e/src/staking_bls.rs`,
`e2e/src/staking.rs` and `genesis-bootstrap`, all of which run the real
predeploys against signatures `blst` made.

Note also that `crates/testing/src/host.rs` routes `static_call` into the same
handler as `call`, so nothing in the unit harness checks that these calls are
static. That is a coverage hole, not a finding about the contract.

### Previous build — 2026-09-07 (second build of the day, superseded)

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
