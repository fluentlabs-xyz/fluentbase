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

## This build

- worktree HEAD: bb7d231d
- worktree was DIRTY at build time (uncommitted work in contracts/staking):
   12 files changed, 856 insertions(+), 1147 deletions(-)
  That is deliberate — the uncommitted delta is what makes the contract match
  this node: it adds `slashEquivocation(uint64,uint32)` and moves the three
  evidence variants from 6 args to 4. A build from a clean HEAD would produce a
  contract the node cannot talk to.
- `fluentbase_contracts_staking.wasm` — 417946 bytes
  (runtime-upgrade payload; the contract compiles it on-chain)
- `fluentbase_contracts_staking.rwasm` — 2904493 bytes
  (genesis install; already compiled with the address-aware config)
