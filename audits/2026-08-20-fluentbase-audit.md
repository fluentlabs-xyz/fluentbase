# Fluentbase Security Audit — 2026-08-20

- **Date:** 2026-08-20 (fixes landed 2026-08-20 → 2026-08-21)
- **Repository:** `fluentlabs-xyz/fluentbase`
- **Audited commit:** `0fd4ded21a684b1bc702eb9ca081959eb7a7df49` (`v1.4.0`)
- **Linear:** parent `FLU-1159`; subtasks `FLU-1160`, `FLU-1161`, `FLU-1163`, `FLU-1164`
- **Fix PRs:** #499, #500, #502
- **Focus:** End-to-end pass on the `v1.4.0` snapshot — Critical/High first (filter:
  Critical and High only), then a follow-up Medium pass on the same snapshot.

## Scope

All Rust/source surfaces under `contracts/` (all system contracts and precompiles,
including the disabled SVM contract) and `crates/` (runtime, REVM/EVM, codec/derive,
SDK/derive, types, crypto, build/contracts/genesis/node/release/testing, SVM groups),
plus Fluent-reached dependency paths: `revm-rwasm` @ `8674122294…`, `reth-core-rwasm`
@ `a1701b3b…`, and `rwasm 0.4.7` (source rev `edfd3da6…`). Source review and
executable tests were the authority.

## Result

| Crit | High | Medium |
| --- | --- | --- |
| 0 | 1 | 2 net-new + 3 revalidated |

The historical Critical/High classes did not reproduce (see Notes).

## Findings

### High

#### FLU-1160 — Universal-token storage writes skip SSTORE write cost & EIP-8037 state gas

- **Severity:** High · **Status:** Accepted/documented · **Linear:** `FLU-1160` · **Fix:** #499
- **Where:** `crates/revm/src/executor.rs:298-367` (preload, read-cost only),
  `:684-686` (`sstore` commit with no gas/state-gas),
  `crates/sdk/src/universal_token/storage.rs`.
- **Impact:** For engine-metered system runtimes — in practice the Universal Token,
  the only stateful one in `EXECUTE_USING_SYSTEM_RUNTIME_ADDRESSES` — storage writes
  are buffered and committed via `JournaledState::sstore` while its `SStoreResult` is
  dropped, so only the up-front SLOAD read price is charged (cold 2100 / warm 100),
  never the EIP-2200 write premium or EIP-8037 state gas. `transfer` to a fresh
  address costs ~cold SLOAD instead of ~`SSTORE_SET` (20000) + state gas; an attacker
  can spray 1-wei transfers/approvals/mints to fresh slots at ~10% of intended cost →
  state-growth DoS. The metadata-write path (`sstore_gas` in `syscall.rs`) and
  `contracts/eip2935` (via `SYSCALL_ID_STORAGE_WRITE`) charge correctly, proving the
  gap is specific to the buffered envelope path.
- **Remediation:** On committing the system-runtime storage diff, use each
  `SStoreResult` to charge EIP-2200 dynamic write cost + EIP-8037 state gas, crediting
  the read cost the preloader already charged; halt out-of-fuel if the frame can't
  afford it. The team accepted and documented the underpricing for this snapshot (PR
  #499) rather than changing the metering.

### Medium

#### FLU-1161 — Consensus-critical rWASM↔REVM resume boundary uses `unwrap`/`expect`/`unreachable!` under `panic="abort"`

- **Severity:** Medium (high blast radius × low likelihood) · **Status:** Fixed · **Linear:** `FLU-1161` · **Fix:** #500
- **Where:** `crates/revm/src/executor.rs:489,542,698-701,826-832`;
  `crates/runtime/src/executor.rs:458-463,526-528`.
- **Impact:** Release profile is `panic = "abort"`, so any of these firing aborts the
  process; sitting on the hottest consensus path driven by cross-boundary data, a
  reachable instance crashes the node and a block-deterministic trigger halts every
  node on that block. Each site is gated by *upstream* invariants (compiler output
  shape, handler ordering, rWASM dependency behavior), not a local check — the class a
  dependency bump can silently break.
- **Remediation:** Convert these sites into graceful deterministic error halts so an
  invariant violation fails the transaction/block instead of aborting the node; add
  regression coverage for malformed interruption params / missing recoverable-runtime
  states; re-audit them in the rwasm upgrade checklist.

#### FLU-1164 — Require canonical release tags before signing artifacts or pushing Docker latest

- **Severity:** Medium · **Status:** Fixed (Final Review) · **Linear:** `FLU-1164` · **Fix:** #502
- **Where:** `.github/workflows/release.yml` (broad `v*` trigger, permissive prefix
  compare, sign/build/release/draft jobs omit `check-version`), `publish.yml`,
  `build-docker.yml`, `docker.yml`, `Makefile:227`.
- **Impact:** `v1.4.0oops` passes `[[ "$tag" == "$cargo_ver"* ]]`, and even a failing
  tag doesn't block the independent build/sign/draft jobs; the Docker workflows treat
  every non-`-rc` `v*` tag as stable and advance public `latest` → signed-but-misleading
  artifacts/metadata and unintended movement of image channels, even when the workflow
  finishes "failed" overall.
- **Remediation:** Centralize canonical tag parsing (permit only `v<workspace-version>`
  and supported prerelease forms); make every sign/upload/publish/draft/Docker-`latest`
  job depend on that validation; add guard tests. Distinct from `FLU-946` (provenance/SBOM)
  and `FLU-1065` (builder-image digest).

#### FLU-1163 — Bound the module cache's secondary code-hash index

- **Severity:** Medium · **Status:** Canceled (accepted hardening item) · **Linear:** `FLU-1163` · **Fix:** —
- **Where:** `crates/runtime/src/module_factory.rs:17-24,26-64,67-87,198-279`;
  `crates/types/src/compilation_cache.rs:25-42,94-105`.
- **Impact:** The 1 GiB compiled-module LRU has an unbounded, process-lifetime
  secondary `code_hash → CompiledModuleCacheKey` index; primary eviction doesn't remove
  the secondary entry → over sustained valid deployment churn a validator retains
  bookkeeping for every distinct executed code hash and RSS grows beyond the 1 GiB
  budget (availability only; no state divergence; hash-only panic not reachable on the
  normal REVM path).
- **Remediation:** Remove the secondary map if hash-only execution is unneeded, or
  prune it atomically on primary eviction and bound its entries; turn a hash-only miss
  into a deterministic error; add a forced-eviction regression test. (The old primary
  cache was fixed historically as `FLU-391`/PR #216; the secondary index is a distinct
  later regression from commit `11389b4f46`.)

#### Revalidated Medium items (not re-filed)

- **Severity:** Medium · **Status:** Open/tracked elsewhere · **Linear:** `FLU-1055`, `FLU-1061`, `FLU-1161`
- **FLU-1055** (ABI decoders panic on out-of-range dynamic offsets/bodies, from the
  2026-08-05 audit) remains reproducible in the current codec derive output.
- **FLU-1061** (unknown RPC transaction type stops consensus block ingestion) is still
  the exact tracker for consensus-subscription termination on an undecodable RPC block.
- **FLU-1161** (above) already covers the rWASM↔REVM resume invariant hardening.

## Notes

**High/Critical regression conclusions.** The historical Critical/High classes did
not reproduce: initial-memory allocation is charged before allocation and aggregate
suspended-frame memory is capped (closes 2026-08-05 `FLU-1046`/`FLU-1047`); precompile
input/gas/curve validation held; direct delegated-runtime calls and static metadata
mutations are blocked; runtime upgrades require both contract authorization and
host-side upgrade authority; interruption state is transaction-scoped; ordinary REVM
execution always supplies bytecode to the module cache. Two items were assessed and
*not* elevated to High: the unbounded secondary module-cache index (filed as Medium
`FLU-1163`) and SVM's incomplete compute-meter adapter (SVM is excluded from the live
system-runtime set).

**Verification (as run).** Passed: `fluentbase-runtime --lib` (72, 1 ignored),
`fluentbase-evm --lib` (2), `fluentbase-revm --lib` (42), codec/SDK/derive/types
suites (codec 76 + integration/doctests, codec-derive 4, SDK 80, sdk-derive-core 136,
types 9), contracts workspace (all enabled; WebAuthn fuel benchmark ignored), build
(19 lib + 23 integration), release verification (38), genesis (1), node (2), rWasm
0.4.7 targeted fuel (6) / interruption (5, 1 ignored) / value-stack-bounds (10).
Not run: packaged rWasm `module::verification`/Wasmtime targets (packaging gap),
standalone SVM build (disabled manifests), Docker-mutating integration tests. Final
worktree clean.

## Re-review checklist for future audits

- Re-check whether system-runtime (Universal Token) storage writes charge SSTORE +
  state gas, or confirm the `FLU-1160` acceptance still holds and is documented.
- Confirm the rWASM↔REVM resume boundary still halts deterministically rather than
  aborting (`FLU-1161`) — re-audit on every `rwasm`/`revm-rwasm` bump.
- Confirm release tag validation is a dependency of every sign/publish/Docker-latest
  job (`FLU-1164`).
- Watch the secondary module-cache index growth (`FLU-1163`) on long-running
  validators.
- Re-confirm `FLU-1055` (codec panic) is closed before relying on derived ABI decoders
  for untrusted input.
