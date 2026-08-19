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

## This build — 2026-08-18

Rebuilt after the on-chain `PK_E` rollback (2026-08-17) and FLU-1134 (committee
retention + the three-structure storage layout). The 2026-08-16 section this
replaces is gone wholesale, per the rule above.

- worktree HEAD: `29ae97ef`
  (`refactor(staking)!: retain epoch committees instead of pruning them (FLU-1134)`)
- worktree was DIRTY at build time, deliberately: the uncommitted delta is
  FLU-1134 phases 2-4 — the `committee_records` / `epoch_index` / `weight_ring`
  split, the ring-miss forfeit, and the deletion of `resolveSigner` and
  `getEpochCommitteeLength`. A build from clean HEAD would produce a contract
  with the OLD `epoch_committees` layout, which this node's reader cannot decode
  (it expects the four-array return with a possibly-empty stakes leg).
- `git status --porcelain contracts/staking` at build time — 10 modified files,
  `10 files changed, 884 insertions(+), 184 deletions(-)`. Indicative only; the
  per-file hashes below are the record, and `README.md` is not compiled:

      M contracts/staking/README.md
      M contracts/staking/src/consensus.rs
      M contracts/staking/src/consts.rs
      M contracts/staking/src/events.rs
      M contracts/staking/src/initializer.rs
      M contracts/staking/src/lib.rs
      M contracts/staking/src/liveness.rs
      M contracts/staking/src/staking.rs
      M contracts/staking/src/storage.rs
      M contracts/staking/src/tests.rs

- SHA-256 of every source file in `contracts/staking/src` as built (dirty
  content, not the committed content — `git show` will NOT reproduce these):

      205ef64f73269a7ed948ba28bbe75c550d3d9a88c375b8327bb1ba42ed07c1bc  src/config.rs
      e518d0f98f62b2853b235c25ada2e85424f76a258dc6f63286d760ca1b3aeaef  src/consensus.rs
      a9c28d73e53546e8138a1418c20b2e273a8c8bca74747cf6553b9054e2758c58  src/consts.rs
      19b44f8d3b01c1c0aba484cd32193bb98675a5398e8e512aefa01c80370b5584  src/events.rs
      3f69dfe02d27be45e6b723e3f128b7049d74abe5c1b6dafac82b5af47d8b5576  src/evidence.rs
      6e19613ec6ba6bc6ffe405b70ad998cc5ba4a1d05c42e851c11fc2f7f38e33f3  src/initializer.rs
      41664ada99d94f2761d511883215d0a36ca2e765b05a410e3f661367a9ba9d20  src/lib.rs
      2cb2284aa8385220c1e23a6c633f98b2954c4d2866f2626d206458fd6883f65e  src/liveness.rs
      87fdd853b1c4d37cbc7421a07c8a6458afc486ff4300289b52015d9958fdf20f  src/math.rs
      f2ecf55fb1ab645347f78753e897bb1b8bc63c8c013967a8edfb9e92e323b74c  src/staking.rs
      94a01f24f8fa81415ad0a9b5a9ded60d05c44d7cdbb1f35ca5a9f7d34a509f53  src/storage.rs
      e0d7192e14f66a0bc9b468d952d7b5809a0986de2362b45a0c16f9e8ca223360  src/tests.rs
      de3751f4f574a205ae8d9aa6cf800d7098f040ac37415debc89cf106634ebe19  src/types.rs
      6c2256b44b8c57d4ea0b34ca2adaf485591c2a0ba9f7d4f123ba6fe7b27f91a6  src/util.rs

  Reproduce with, from `contracts/staking`:

      find src -name '*.rs' | sort | xargs sha256sum

- `fluentbase_contracts_staking.wasm` — 414,513 bytes (was 425,932 on 2026-08-16)
  `8f5895a586f172afb97a4894ccf66f18367118802f895ca7424b88426bf79797`
- `fluentbase_contracts_staking.rwasm` — 2,854,198 bytes (was 2,950,957)
  `f30deb0d6a9a0d15a3ac0344076139f301b9d2a29b8cbb1281a993d5df33529e`
- Both shrank: the beacon-key layer came out and the committee storage lost
  `prune_committees`, `resolveSigner` and `getEpochCommitteeLength`.
- Selector scan on the `.rwasm`, extended beyond the four the check above lists
  because this build DELETES selectors as well as adding them — an absence is as
  much a correctness claim as a presence:

      present, 1 each:  recordProduction commitEpochCommittee slashEquivocation
                        producedAt getEpochCommitteeWithStakes getEpochCommittee
                        getEpochRewards getDkgQual
      absent, 0 each:   commitEpochBeaconKey getEpochBeaconKey
                        resolveSigner getEpochCommitteeLength

  The two beacon-key selectors are the 2026-08-16 staleness marker; the other two
  are FLU-1134's deletions. A blob carrying any of the four is not this build.
- **Determinism re-confirmed on this build.** Two release builds an hour apart,
  separated only by doc-comment edits to `consensus.rs` / `storage.rs` /
  `staking.rs`, produced BIT-IDENTICAL `.rwasm`
  (`f30deb0d…`, 2026-08-17 23:48 and 2026-08-18 00:44). Consistent with the
  earlier data point below: source hashes that move without the blob moving mean
  the delta was comment-only.
- **NOT yet done for this build:** the smoke docker image has not been rebuilt,
  so no golden snapshot is trustworthy against these blobs yet, and the devnet has
  not been run at all against FLU-1134.

- **Reproducibility data point, from the 2026-08-16 build** (kept because it is
  the original observation the note above re-confirms): those blobs were built
  twice from this worktree, the second time after a doc-comment-only edit to
  `src/consensus.rs`. Both builds produced BIT-IDENTICAL `.wasm` and `.rwasm`. So
  the toolchain is deterministic for this crate, and a source hash that moves
  without the blob moving means the change emitted no code — worth checking
  before assuming a rebuild is needed.

### Previous build — 2026-08-08 (superseded)

Same HEAD `bb7d231d`, dirty with the same 12 files at an earlier content
(`12 files changed, 856 insertions(+), 1147 deletions(-)`), no source hashes
recorded. `fluentbase_contracts_staking.wasm` 417,946 bytes;
`fluentbase_contracts_staking.rwasm` 2,904,493 bytes; no SHA-256 recorded. It
carries no `commitEpochBeaconKey` / `getEpochBeaconKey` — like the post-rollback
build, though it predates several other node-side changes and is NOT a substitute
for one.
