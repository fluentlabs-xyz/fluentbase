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

Both blobs are **gitignored** (`.gitignore:34`), so git records nothing about
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

## This build — 2026-09-04

Rebuilt for Э0.2 (`.dpos-study/PLAN.md`): `commitEpochCommittee` no longer reverts
`ERR_COMMITTEE_TOO_SMALL` when the selection falls below `MIN_COMMITTEE_LENGTH`.
It carries the previous epoch's committee forward instead — record pointer,
length and re-stamped weight frame — writes `dkgQual[target] = false`, advances
the cursor, and emits the new `CommitteeCarriedOver(uint64 indexed epoch, uint32
eligible, uint32 members)`. The revert survives only at `target == 0`. This
closes R-111 / R-112 / K-3, the one path on which a single honest `undelegate`
killed every node at the next epoch boundary with no way back (reproduced on the
stand: `.dpos-study/EXPERIMENTS.md` §5, E2 and E5).

- worktree HEAD: `bc42042a` (`wip`)
- **HEAD moved since the 2026-08-18 section this replaces.** That section
  described a build from `29ae97ef` DIRTY with 10 files (the FLU-1134 phases 2-4
  delta). Those files are now COMMITTED: the per-file hashes below for the eleven
  files this build did not touch are byte-identical to the ones recorded there,
  and `git show HEAD:contracts/staking/src/{consensus,events,tests}.rs | sha256sum`
  reproduces that section's recorded hashes for the other three. So the baseline
  of this build is exactly the source that produced the previous blob, and the
  whole delta between the two blobs is the three dirty files below.
- worktree DIRTY at build time, deliberately: the uncommitted delta is Э0.2 and
  nothing else. `git status --porcelain contracts/staking` — 3 modified files,
  `3 files changed, 358 insertions(+), 18 deletions(-)`:

      M contracts/staking/src/consensus.rs   (carry_committee_forward + write_ring split)
      M contracts/staking/src/events.rs      (CommitteeCarriedOver)
      M contracts/staking/src/tests.rs       (the carry-over behaviour test)

- SHA-256 of every source file in `contracts/staking/src` as built (dirty
  content, not the committed content — `git show` will NOT reproduce the three
  starred ones):

      205ef64f73269a7ed948ba28bbe75c550d3d9a88c375b8327bb1ba42ed07c1bc  src/config.rs
      52353942b8dbd3a45c7c04a3ede782d7c6aaca5971f0a866ffd6e76980b5a442  src/consensus.rs   *
      a9c28d73e53546e8138a1418c20b2e273a8c8bca74747cf6553b9054e2758c58  src/consts.rs
      3a5c920cfc1f27c2cbcb5838db3717def3994d7e1fe690d13daf345f68645af6  src/events.rs      *
      3f69dfe02d27be45e6b723e3f128b7049d74abe5c1b6dafac82b5af47d8b5576  src/evidence.rs
      6e19613ec6ba6bc6ffe405b70ad998cc5ba4a1d05c42e851c11fc2f7f38e33f3  src/initializer.rs
      41664ada99d94f2761d511883215d0a36ca2e765b05a410e3f661367a9ba9d20  src/lib.rs
      2cb2284aa8385220c1e23a6c633f98b2954c4d2866f2626d206458fd6883f65e  src/liveness.rs
      87fdd853b1c4d37cbc7421a07c8a6458afc486ff4300289b52015d9958fdf20f  src/math.rs
      f2ecf55fb1ab645347f78753e897bb1b8bc63c8c013967a8edfb9e92e323b74c  src/staking.rs
      94a01f24f8fa81415ad0a9b5a9ded60d05c44d7cdbb1f35ca5a9f7d34a509f53  src/storage.rs
      fe90ab03374c81debe3442ec6d4cac78470d523c2ada0052c86751db3373eddf  src/tests.rs       *
      de3751f4f574a205ae8d9aa6cf800d7098f040ac37415debc89cf106634ebe19  src/types.rs
      6c2256b44b8c57d4ea0b34ca2adaf485591c2a0ba9f7d4f123ba6fe7b27f91a6  src/util.rs

  Reproduce with, from `contracts/staking`:

      find src -name '*.rs' | sort | xargs sha256sum

- `fluentbase_contracts_staking.wasm` — 416,781 bytes (was 414,513 on 2026-08-18)
  `c350bffb97e667fbcc152903dd534cacac2fbf112d213d4defe6ec56d4c3d91b`
- `fluentbase_contracts_staking.rwasm` — 2,868,708 bytes (was 2,854,198)
  `988955e5fea07cc65d543b74933e68a7451a1375ea0fac98d980c198660697f4`
- Both grew (+2,268 / +14,510): the carry-over branch, the ring-write split and
  one more event descriptor. Nothing was deleted.
- `cargo test --lib` in `contracts/staking`: **162 passed, 0 failed** (161 before,
  plus `a_short_selection_carries_the_previous_committee_instead_of_stopping_the_chain`).
  That test was confirmed to FAIL against the pre-change contract — the branch was
  temporarily reverted to the old `revert_with` and the test reddened on the
  "a short selection must not revert" assertion — so it is not vacuous.
- Selector scan on the `.rwasm`. The four the general check above lists, plus the
  reads the node makes and the four absences the previous section established.
  `getEpochCommittee` is `0x80b562de` and `getEpochRewards` is `0x54c3e84b`
  (`cast sig`); the previous section named neither, so a scan copied from it would
  silently skip them:

      present, 1 each:  recordProduction (0x1752910e) commitEpochCommittee (0xe505b249)
                        slashEquivocation (0xdc6fb3f2) producedAt (0x91c7d453)
                        getEpochCommitteeWithStakes (0xa4d160c1)
                        getEpochCommittee (0x80b562de) getEpochRewards (0x54c3e84b)
                        getDkgQual (0x2660899f)
      absent, 0 each:   commitEpochBeaconKey (0x6ece9cb1) getEpochBeaconKey (0xc9adaf5c)
                        resolveSigner getEpochCommitteeLength

  `CommitteeCarriedOver` is an EVENT, so it has no selector to scan for; its
  presence is pinned by the contract test rather than by this scan.
- **The smoke docker image was rebuilt after this swap and the devnet was run
  against it** — see the Э0 log (`.dpos-study/E0-LOG.md`) for what was observed.
  Any golden snapshot taken before 2026-09-04 is stale (`golden.py::_image_id`).

### Previous build — 2026-08-08 (superseded)

Same HEAD `bb7d231d`, dirty with the same 12 files at an earlier content
(`12 files changed, 856 insertions(+), 1147 deletions(-)`), no source hashes
recorded. `fluentbase_contracts_staking.wasm` 417,946 bytes;
`fluentbase_contracts_staking.rwasm` 2,904,493 bytes; no SHA-256 recorded. It
carries no `commitEpochBeaconKey` / `getEpochBeaconKey` — like the post-rollback
build, though it predates several other node-side changes and is NOT a substitute
for one.
