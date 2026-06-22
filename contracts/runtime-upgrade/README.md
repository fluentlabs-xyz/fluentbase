# Runtime Upgrade

Privileged system contract for replacing Fluent runtime/system contract bytecode.
Runtime upgrades are consensus-critical, so this contract keeps authorization and audit metadata
explicit in calldata/events while the host still enforces the native upgrade syscall boundary.

## Entry Points

### `upgradeTo(address target, uint256 genesisHash, string genesisVersion, bytes wasmBytecode)`

Owner-only direct upgrade path.

The contract validates the submitted WASM bytes, compiles them to rWasm for `target`, invokes the
native runtime-upgrade syscall, then emits `RuntimeUpgraded(target, genesisHash, genesisVersion,
codeHash)`.

### `recompile(address target)`

Owner-only maintenance path for already deployed WASM bytecode.

The contract reads bytecode from `target` with `code_size`/`code_copy`, requires the loaded length
to match the reported size, recompiles the bytecode through the same install path as `upgradeTo`,
and emits `ContractRecompiled(target, codeHash)`.

### `planUpgrade(uint256 genesisHash, string genesisVersion, address[] targets, bytes32[] wasmHashes, address upgrador)`

Owner-only planning path for release upgrades that should be executable by a delegated account.

The plan stores parallel `(target, wasmHash)` pairs plus shared release metadata and a single
authorized `upgrador`. The contract rejects empty plans, mismatched target/hash array lengths, zero
targets, zero hashes, duplicate targets, and a zero upgrador.

Target binding is intentional: approving a raw WASM hash alone would let the delegated upgrador
install approved bytecode at the wrong system address. A planned upgrade is valid only for the exact
target/hash pair approved by the owner.

Calling `planUpgrade` replaces any previous plan.

### `upgradeToPlanned(address target, bytes wasmBytecode)`

Delegated execution path for planned upgrades.

Only the stored `upgrador` can call this method. The contract hashes `wasmBytecode`, checks that
`(target, keccak256(wasmBytecode))` exists in the current plan, runs the same compile/install path
as `upgradeTo`, removes the consumed pair, and emits `RuntimeUpgraded`.

Removing the pair prevents replaying the same planned target/hash entry after it has been installed.

## Trust Model

- `owner` can perform direct upgrades, recompile existing targets, and replace the current plan.
- `upgrador` can execute only owner-approved target/hash pairs from the current plan.
- Host-side syscall enforcement remains the final boundary: `SYSCALL_ID_UPGRADE_RUNTIME` must only
  be reachable through the runtime-upgrade precompile execution path.

## Operational Notes

- Off-chain release tooling must hash the exact raw WASM bytes that will be submitted to
  `upgradeToPlanned`.
- Plans are release-scoped: `genesisHash` and `genesisVersion` are emitted audit metadata shared by
  every planned entry.
- Very large owner-submitted plans increase linear lookup cost during execution and quadratic
  duplicate-target validation during planning. Keep plans bounded to the release batch.
