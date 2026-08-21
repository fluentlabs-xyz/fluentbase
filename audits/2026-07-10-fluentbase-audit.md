# Fluentbase Security Audit — 2026-07-10

- **Date:** 2026-07-10 (fixes landed 2026-07-14 → 2026-07-17)
- **Repository:** `fluentlabs-xyz/fluentbase` (audited commit
  `99ba646fbb64906305149fbd2c444d06bda974e4`); rWasm reviewed as a dependency
  (`fluentlabs-xyz/rwasm` @ `46573169788f2b2b8b051b5443d398f3680146e9`)
- **Audited commit:** `99ba646fbb64906305149fbd2c444d06bda974e4`
- **Linear:** parent items `FLU-935`…`FLU-946` (Fluentbase); `FLU-947`…`FLU-952`
  (rWasm dependency)
- **Fix PRs:** Fluentbase #457–#467; rWasm #162, #163, #164
- **Focus:** Host runtime, EVM/REVM, genesis, runtime-upgrade CLI, precompiles,
  and the rWasm dependency reached during contract execution.

## Scope

Fluentbase host findings use the `H-01` / `M-0x` / `L-0x` / `I-0x` series; the
rWasm dependency findings produced in the same pass use `RWASM-01…06`. Both are
recorded here because this was a single combined pass; the rWasm items are kept
for continuity with the report maintained alongside the rWasm repository.

## Result

| Series | Crit | High | Medium | Low | Info |
| --- | --- | --- | --- | --- | --- |
| Fluentbase (H/M/L/I) | 0 | 1 | 4 | 2 | 5 |
| rWasm dependency (RWASM-01…06) | 1 | 2 | 2 | 1 | — |

## Findings

### Fluentbase host findings

#### H-01 — Meter bulk Wasm memory and enforce transaction-wide memory limits

- **Severity:** High · **Status:** Fixed · **Linear:** `FLU-935` · **Fix:** #459
- **Where:** `crates/sdk/src/types/rwasm.rs`, `contracts/wasm/src/lib.rs`,
  `crates/runtime/src/executor.rs`, `crates/runtime/src/syscall_handler/host/exec.rs`.
- **Impact:** Untrusted Wasm compiled with rWasm's default
  `consume_fuel_for_bulk_ops = false`, so `memory.grow/fill/copy/init` and table
  ops got fixed operator charges instead of size-proportional cost; suspended
  parent runtimes stay resident during nested execution, letting transaction memory
  exceed the per-runtime cap → validator OOM / block stalls / liveness degradation.
- **Remediation:** Enable bulk-op metering on every untrusted path; add a
  transaction-wide memory budget covering active + suspended runtimes; make
  allocation failure deterministic.

#### M-01 — Make runtime-upgrade CLI fail closed and abort partial batches

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-936` · **Fix:** #461
- **Where:** `bins/runtime-upgrade/main.rs`.
- **Impact:** The CLI could report success after failed/partial upgrades (malformed
  modules skipped, missing modules → `RwasmModule::default()`, `DONE` printed
  without checking `receipt.status`, `Ok(None)` treated as success, post-upgrade
  mismatch only warns) → mixed system-runtime versions while the operator sees
  exit 0.
- **Remediation:** Preflight-reject absent/malformed/oversized artifacts; require
  mined receipt `status == 1`; treat dropped/absent/timeout/mismatch as fatal; stop
  on first failure with non-zero exit; verify all code hashes before broadcasting.

#### M-02 — Make genesis artifacts reproducible and bind releases to immutable hashes

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-937` · **Fix:** #460
- **Where:** `crates/genesis/build.rs`, `.github/workflows/release.yml`.
- **Impact:** The devnet/testnet genesis timestamp derived from `SystemTime::now()`,
  so the same commit/tag can produce a different genesis JSON/hash on rebuild;
  operators with nominally identical artifacts can initialize different chains.
- **Remediation:** Replace wall-clock fields with reviewed immutable manifest
  values; publish/verify the expected uncompressed genesis hash before signing;
  refuse publication on rebuild mismatch; signed manifest binding
  network/version/commit/genesis hash.

#### M-03 — Return canonical EVM code hash for cold EXTCODEHASH

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-938` · **Fix:** #457, #458
- **Where:** `crates/revm/src/syscall.rs`.
- **Impact:** `CODE_HASH` used `load_..._skip_cold_load(..., load_code = false)`;
  on a cold load Reth supplies `code: None`, so the fallback returned the wrapper
  bytecode hash, and after `EXTCODESIZE`/`EXTCODECOPY` warmed the account a later
  `EXTCODEHASH` could return the original EVM hash — an access-order-dependent
  result that breaks code-hash allowlists, proxy/factory validation, etc.
- **Remediation:** Load code in the `CODE_HASH` path (or expose the original EVM
  hash as first-class metadata); return one canonical hash independent of access
  order.

#### M-04 — Evict cached system-runtime instances after every abnormal exit

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-939` · **Fix:** #463
- **Where:** `crates/runtime/src/runtime/system_runtime.rs`, `crates/sdk/src/entrypoint.rs`.
- **Impact:** Thread-local Wasmtime instances (store/memory/globals/allocator
  persist) were evicted after a non-OK result and `OutOfFuel`, but other Wasmtime
  traps only set `UnexpectedFatalExecutionFailure` and returned without eviction →
  a later call on the same worker thread can inherit dirty state or a persistent
  trap.
- **Remediation:** Evict after every trap/abnormal return; add an RAII health guard;
  include metering mode + compilation config in the cache key; bound cached instance
  count.

#### L-01 — Implement or fork-gate correct BLOBBASEFEE semantics

- **Severity:** Low · **Status:** Fixed · **Linear:** `FLU-940` · **Fix:** —
- **Where:** `crates/evm/src/host.rs`.
- **Impact:** Default REVM context selects Osaka (where `BLOBBASEFEE` is active) but
  the host returned ordinary `block_base_fee()` instead of the blob gas price →
  non-Ethereum value for ordinary contracts. (Deeper delegated-EVM blob finding
  tracked later as `FLU-1057` in the 2026-08-05 audit.)
- **Remediation:** Supply correct excess-blob-gas fields and derive the Ethereum
  blob gas price, or fork-gate the opcode; re-enable current-fork differential
  tests.

#### L-02 — Clarify or complete WebAuthn relying-party validation

- **Severity:** Low / design risk · **Status:** Fixed · **Linear:** `FLU-941` · **Fix:** #464
- **Where:** WebAuthn precompile (`contracts/webauthn`).
- **Impact:** Verifies challenge, type, UP/UV flags, and P-256 signature but not
  expected RP-ID hash, origin policy, counter, or extensions, and the ABI provides
  no expected RP ID/origin — integrators may treat a selective assertion-signature
  primitive as a complete relying-party verifier.
- **Remediation:** Rename/document as a selective verifier, or add expected RP-ID
  hash + origin policy inputs and enforce them on-chain (the PR added strict
  policy checks).

#### I-01 — Remove or implement the active OAuth2 verifier placeholder

- **Severity:** Informational · **Status:** Canceled · **Linear:** `FLU-942` · **Fix:** —
- **Where:** `contracts/oauth2/src/lib.rs` returns
  `ExitCode::UnreachableCodeReached` while its address is in the
  system-precompile/genesis surface.
- **Impact:** Docs/integrations could treat a non-functional address as
  operational.
- **Remediation:** Implement or fork-gate/remove the address. (Ticket later
  canceled; no fix landed under this ID.)

#### I-02 — Complete and gate root/STF interruption continuation

- **Severity:** Informational (release-blocking before STF/zkVM) · **Status:** Fixed · **Linear:** `FLU-943` · **Fix:** #462
- **Where:** root/STF interruption path (`syscall_exec_continue`).
- **Impact:** Contained `unimplemented!` with unfinished root serialization; no
  default-path user panic confirmed, but unsafe to enable for STF/zkVM mode.
- **Remediation:** Feature/fork-gate activation until complete; remove
  `unimplemented!`/placeholder serialization in the enabled mode.

#### I-03 — Apply full storage gas semantics to metadata storage syscalls

- **Severity:** Informational / secondary · **Status:** Fixed · **Linear:** `FLU-944` · **Fix:** #465
- **Where:** metadata storage read/write handlers (journal `sload`/`sstore`).
- **Impact:** No cold/warm access, dynamic SSTORE cost, stipend checks, or refunds;
  reachability primarily via the excluded SVM path.
- **Remediation:** Apply EVM-equivalent access + dynamic write pricing; enforce
  static-call restrictions and refunds; gate SVM enablement on tests.

#### I-04 — Include compilation configuration fingerprints in runtime and build caches

- **Severity:** Informational / hardening · **Status:** Fixed · **Linear:** `FLU-945` · **Fix:** #466
- **Where:** system-runtime instance cache + genesis compilation cache.
- **Impact:** Cached by code hash / Wasm bytes even though behavior depends on
  address-derived metering mode, memory limits, linker config, engine/backend, and
  fork settings.
- **Remediation:** Key caches by `(code hash, engine/backend, metering mode,
  limits, linker version, config/fork version)` and `(Wasm hash, config
  fingerprint)`; invalidate across upgrades/forks.

#### I-05 — Harden release and CI supply-chain integrity

- **Severity:** Informational / supply-chain · **Status:** Fixed · **Linear:** `FLU-946` · **Fix:** #467
- **Where:** release/CI workflows.
- **Impact:** Mutable action/toolchain/image/branch/tag references; `--locked` not
  consistently required; no complete immutable provenance chain. (Further tag
  canonicalization gaps found later as `FLU-1164` in the 2026-08-20 audit.)
- **Remediation:** Pin Actions by SHA; pin toolchains/tools/images by version +
  digest; use `--locked`; generate provenance + SBOM; bind all critical inputs and
  output hashes in a signed manifest.

### rWasm dependency findings

#### RWASM-01 — Verify serialized rWasm before native execution

- **Severity:** Critical · **Status:** Fixed · **Linear:** `FLU-947` · **Fix:** rwasm #162
- **Where:** `RwasmModule::new_checked` (`src/module/mod.rs`); executor pointer use
  in `vm/engine.rs`, `vm/instr_ptr.rs`, `vm/value_stack.rs`.
- **Impact:** `new_checked` verified only bincode-decodability, not the semantic
  invariants the native VM relies on (`source_pc`, branch/call/table targets, local
  depths, syscall indexes). A malformed artifact can reach OOB pointer operations
  through a safe public API — crashes, OOB read/write, UB, potentially native code
  execution.
- **Remediation:** Sealed verified module type + complete bytecode verifier;
  native execution only for verified modules; raw-artifact fuzzing. (The verifier's
  own gaps were then found in the 2026-08-07 rWasm audit, `FLU-1094`/`FLU-1097`.)

#### RWASM-02 — Bind syscalls to a module-specific capability manifest

- **Severity:** High · **Status:** Canceled · **Linear:** `FLU-948` · **Fix:** —
- **Where:** serialized format (`src/module/mod.rs`), `invoke_syscall`
  (`src/vm/executor.rs`), `src/vm/import_linker.rs`.
- **Impact:** The format retains no authoritative import manifest; a crafted
  artifact can encode any `sys_func_idx` registered in the runtime linker,
  bypassing capability separation where the linker holds privileged functions.
- **Remediation:** Per-module verified syscall manifest checked before execution.
  (Ticket canceled; mitigated in practice by a linker containing only permitted
  functions.)

#### RWASM-03 — Meter bulk operations and make memory growth fallible

- **Severity:** High · **Status:** Fixed · **Linear:** `FLU-949` · **Fix:** rwasm #163
- **Where:** `src/compiler/config.rs`, `src/compiler/translator.rs`,
  `src/isa/memory.rs`, `src/isa/table.rs`, `src/vm/memory.rs`.
- **Impact:** `consume_fuel_for_bulk_ops = false` by default → bulk ops charged only
  fixed cost; `GlobalMemory::grow` via `BytesMut::resize` can abort the process
  instead of returning `memory.grow == -1`. This is the rWasm-side root cause of
  H-01.
- **Remediation:** Proportional charging as the secure default with checked `u64`
  arithmetic; fallible allocation returning `u32::MAX`; aggregate memory budget;
  keep both strategy costs aligned.

#### RWASM-04 — Bound serialized-module decoding allocations

- **Severity:** Medium · **Status:** Canceled · **Linear:** `FLU-950` · **Fix:** —
- **Where:** `src/module/mod.rs` (`new_checked` bincode decode).
- **Impact:** No total/container size limit; attacker-sized code/data/element/hint
  vectors trigger reservation/capacity-overflow/OOM before any semantic verifier
  runs (bypasses runtime fuel).
- **Remediation:** Per-section limits + bounded decoding. (Ticket canceled; the
  same class was re-found and fixed as `FLU-1096` in the 2026-08-07 rWasm audit.)

#### RWASM-05 — Bind suspended execution state to its instance

- **Severity:** Medium · **Status:** Canceled · **Linear:** `FLU-951` · **Fix:** —
- **Where:** `src/vm/store.rs`, `src/vm/instance.rs`, `src/vm/engine.rs`.
- **Impact:** `resume` doesn't verify the `resumable_context` belongs to the
  resuming instance/module; overlapping execution guarded only by `debug_assert!`;
  resuming without a context reaches `unreachable!` → wrong-instance resume or
  crash.
- **Remediation:** Bind `ReusableContext` to a stable identity; typed errors
  (`AlreadySuspended`/`NotSuspended`/`WrongInstance`/`IncompatibleModule`). (Ticket
  canceled under this ID.)

#### RWASM-06 — Reject partially encoded `source_pc` fields

- **Severity:** Low · **Status:** Fixed · **Linear:** `FLU-952` · **Fix:** rwasm #164
- **Where:** `src/module/mod.rs` (`RwasmModuleInner::decode` legacy fallback).
- **Impact:** `UnexpectedEnd` on the final `source_pc` is treated as legacy with a
  `debug_assert_eq!` on the missing-byte count; in release a partially-present field
  can be accepted as legacy → truncating an artifact silently changes its entry
  point to zero while still being accepted.
- **Remediation:** Apply the legacy fallback only when zero bytes are present;
  reject partial fields in debug and release; add an explicit format/version
  discriminator.

## Notes

Four of the six rWasm dependency items were canceled under their July IDs
(RWASM-02/04/05) or fixed (RWASM-01/03/06); the decode-bounds class (RWASM-04) and
verifier-completeness class (RWASM-01) were subsequently re-found and driven to
closure in the 2026-08-07 rWasm audit.

## Re-review checklist for future audits

- Confirm bulk-op metering is enabled by default and a transaction-wide memory
  budget covers suspended frames (H-01 / RWASM-03).
- Confirm `EXTCODEHASH` is access-order-independent for delegated EVM contracts
  (M-03).
- Confirm the runtime-upgrade CLI fails closed on every partial-batch path (M-01).
- Confirm the rWasm semantic verifier is applied before native execution, and
  re-check its completeness against `FLU-1094`/`FLU-1097` (RWASM-01).
- Re-open the canceled hardening items (RWASM-02 capability manifest, RWASM-04
  decode bounds, RWASM-05 suspended-state binding) if the executable format or
  raw-artifact entry points change.
