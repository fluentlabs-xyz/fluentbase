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
contains either selector is the 2026-08-16 build and is STALE — see the rebuild
note below.

## This build — 2026-08-16 (STALE — rebuild required)

**STALE as of 2026-08-17.** The beacon-key rollback edited
`consts.rs` / `storage.rs` / `events.rs` / `lib.rs` / `consensus.rs` /
`tests.rs` / `README.md` in the worktree, so every source hash, size and SHA-256
in this section is from a source tree that no longer exists, and the blobs beside
this file still CONTAIN `commitEpochBeaconKey` / `getEpochBeaconKey`. They must be
rebuilt (build command above), copied here by hand, and the smoke image rebuilt —
it bakes the genesis. Replace this whole section wholesale when you do.

- worktree HEAD: `bb7d231d`
- worktree was DIRTY at build time. That is deliberate: the uncommitted delta is
  what makes the contract match this node. It adds
  `slashEquivocation(uint64,uint32)` and moves the three evidence variants from 6
  args to 4. A build from a clean HEAD would produce a contract the node cannot
  talk to.
- `git status --porcelain contracts/staking` at build time — 12 modified files,
  `12 files changed, 1273 insertions(+), 1179 deletions(-)`. The diffstat is
  indicative only; the per-file source hashes below are the authoritative
  record, and `README.md` is in the list but is not compiled, so it cannot move
  the blob:

      M contracts/staking/README.md
      M contracts/staking/src/config.rs
      M contracts/staking/src/consensus.rs
      M contracts/staking/src/consts.rs
      M contracts/staking/src/events.rs
      M contracts/staking/src/evidence.rs
      M contracts/staking/src/initializer.rs
      M contracts/staking/src/lib.rs
      M contracts/staking/src/staking.rs
      M contracts/staking/src/storage.rs
      M contracts/staking/src/tests.rs
      M contracts/staking/src/types.rs

- SHA-256 of every source file in `contracts/staking/src` as built (dirty
  content, not the committed content — `git show` will NOT reproduce these):

      205ef64f73269a7ed948ba28bbe75c550d3d9a88c375b8327bb1ba42ed07c1bc  src/config.rs
      0e3ea8429fd93a5c0a6b79f26e302a2578382fe12da193df9da570968f89438b  src/consensus.rs
      a277320be1a2feb49c70bd91fda3ab3c12610c35deafae0efc205266814809b2  src/consts.rs
      37bb6514f95a44b2327b72baf7a68ee9fec21803786c01636627f71c5bd511cd  src/events.rs
      3f69dfe02d27be45e6b723e3f128b7049d74abe5c1b6dafac82b5af47d8b5576  src/evidence.rs
      6e19613ec6ba6bc6ffe405b70ad998cc5ba4a1d05c42e851c11fc2f7f38e33f3  src/initializer.rs
      ed90bab3f5ce5150aed3a12831be8762b5d3f5bfc5064e06f44d0fd02d7219e6  src/lib.rs
      98c034427e88fe85923b724de90591fb8cdfbf7b000bce06d3544ec570a12948  src/liveness.rs
      87fdd853b1c4d37cbc7421a07c8a6458afc486ff4300289b52015d9958fdf20f  src/math.rs
      5e0784a467b4b7abc5a6c1375e442fd488840c7e1ebb896f03dbbb74a1c1234a  src/staking.rs
      529a4d8a69eef8598a4f4a1cfafc5585fef0aabb4529b58e2c80959c21ab4b67  src/storage.rs
      da4dcce271edeaf3d65f175da484b9ba90dd6c6121debfa8d8f13e37427aefb9  src/tests.rs
      de3751f4f574a205ae8d9aa6cf800d7098f040ac37415debc89cf106634ebe19  src/types.rs
      6c2256b44b8c57d4ea0b34ca2adaf485591c2a0ba9f7d4f123ba6fe7b27f91a6  src/util.rs

  Reproduce with, from `contracts/staking`:

      find src -name '*.rs' | sort | xargs sha256sum

- `fluentbase_contracts_staking.wasm` — 425,932 bytes
  (runtime-upgrade payload; the contract compiles it on-chain)
  `f7eb669f6c6af5e5052c7168f5bbdc1f732fd7a15adfcb13b1348428011f49ba`
- `fluentbase_contracts_staking.rwasm` — 2,950,957 bytes
  (genesis install; already compiled with the address-aware config)
  `f4a530f988a02a4838f37f20d055a8bd39c13545a4ae230a34a255297fc321d8`
- Selector scan on the `.rwasm` at build time: all six selectors of that build
  present (the four above plus the two beacon-key ones, since removed), exactly
  one occurrence each.
- **Reproducibility data point:** these blobs were built twice from this
  worktree, the second time after a doc-comment-only edit to `src/consensus.rs`
  (hence the hash above differing from the first build's record). Both builds
  produced BIT-IDENTICAL `.wasm` and `.rwasm`. So the toolchain is deterministic
  for this crate, and a source hash that moves without the blob moving means the
  change emitted no code — which is worth checking before assuming a rebuild is
  needed.

### Previous build — 2026-08-08 (superseded)

Same HEAD `bb7d231d`, dirty with the same 12 files at an earlier content
(`12 files changed, 856 insertions(+), 1147 deletions(-)`), no source hashes
recorded. `fluentbase_contracts_staking.wasm` 417,946 bytes;
`fluentbase_contracts_staking.rwasm` 2,904,493 bytes; no SHA-256 recorded. It
carries no `commitEpochBeaconKey` / `getEpochBeaconKey` — like the post-rollback
build, though it predates several other node-side changes and is NOT a substitute
for one.
