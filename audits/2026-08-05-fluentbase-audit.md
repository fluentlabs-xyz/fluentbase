# Fluentbase Security Audit — 2026-08-05

- **Date:** 2026-08-05 (fixes landed 2026-08-06 → 2026-08-07)
- **Repository:** `fluentlabs-xyz/fluentbase`
- **Audited commit:** `19a1fe0f9ff9616684e157a89f6eb8b313f9f980`
- **Linear:** parent `FLU-1045`; subtasks `FLU-1046`…`FLU-1079`
- **Fix PRs:** one per finding, landed on the finding's Linear issue
- **Focus:** Full rWasm/EVM pass — 15 `crates/*` audits + 18 `contracts/*` audits;
  SVM and inactive code excluded; `contracts/nitro` excluded by request.

## Scope

Independent per-crate and per-contract source reviews with an adversarial
adjudication pass. 49 raw confirmed candidates were reduced to 34 unique root
causes (5 duplicate instances merged; 10 downgraded/rejected). Contract scopes
clean at Medium+: `bls12381`, `bn256`, `ecrecover`, `eip2935`, `identity`, `kzg`,
`modexp`, `oauth2`, `ripemd160`, `sha256`, `wasm`.

## Result

| Crit | High | Medium | Total subtasks |
| --- | --- | --- | --- |
| 0 | 6 | 28 | 34 |

All findings are Fixed unless marked otherwise (`FLU-1058` was Canceled).

## Findings

### High

#### FLU-1046 — Bound aggregate rWasm output to prevent validator memory exhaustion

- **Severity:** High · **Status:** Fixed · **Linear:** `FLU-1046` · **Fix:** —
- **Where:** `crates/runtime/src/syscall_handler/host/write_output.rs`,
  `.../forward_output.rs`, `crates/runtime/src/executor.rs`.
- **Impact:** `_write`/`_forward_output` each validate their range but append to one
  unconstrained host `Vec<u8>`; at 3 gas/word a guest can grow host output toward
  1 GiB under the block gas limit, and allocator failure isn't a deterministic guest
  error → permissionless validator memory exhaustion.
- **Remediation:** Consensus-defined aggregate output cap checked with overflow-safe
  arithmetic before allocate/copy; deterministic VM error when exceeded.

#### FLU-1047 — Meter initial rWasm memory and cap aggregate suspended-frame memory

- **Severity:** High · **Status:** Fixed · **Linear:** `FLU-1047` · **Fix:** —
- **Where:** `crates/runtime/src/runtime/contract_runtime.rs`,
  `crates/runtime/src/executor.rs`, `crates/runtime/src/syscall_handler/host/exec.rs`.
- **Impact:** A guest can declare ~1024 initial pages; rWasm runs the initial
  `memory.grow` with fuel injection disabled during instantiation (~64 MiB before
  charging), and suspended runtimes stay in `recoverable_runtimes` with no aggregate
  budget → memory exhaustion before per-instance limits.
- **Remediation:** Meter/reject initial growth before allocation; aggregate memory
  budget over active + suspended frames; fallible reservation.

#### FLU-1048 — Authenticate runtime-upgrade release artifacts before signing upgrades

- **Severity:** High · **Status:** Fixed · **Linear:** `FLU-1048` · **Fix:** —
- **Where:** `bins/runtime-upgrade/main.rs`; producer `.github/workflows/release.yml`.
- **Impact:** The privileged CLI accepts an unsigned same-name cached JSON or the
  gzip without verifying the detached signature/manifest; the selected hint WASM is
  then encoded into owner-gated calls and broadcast → cache substitution / compromised
  asset becomes operator-signed system code.
- **Remediation:** Verify pinned signer + signature (or trusted manifest) before use;
  bind to network/release/filename/digest; fail closed before wallet access.

#### FLU-1049 — Implement fail-closed genesis signature verification for built-in networks

- **Severity:** High · **Status:** Fixed · **Linear:** `FLU-1049` · **Fix:** —
- **Where:** `crates/node/src/utils.rs`, `crates/node/src/chainspec.rs`.
- **Impact:** `verify_detached_signature` ignores both paths and returns success;
  devnet/testnet lack an independent genesis hash → substituted artifacts control
  genesis allocations and system code.
- **Remediation:** Real verification against a pinned key, bound to the exact
  compressed artifact; no decompression/parse before authentication.

#### FLU-1050 — Revoke planned runtime-upgrade authority on ownership transitions

- **Severity:** High · **Status:** Fixed · **Linear:** `FLU-1050` · **Fix:** —
- **Where:** `contracts/runtime-upgrade/src/lib.rs`.
- **Impact:** `changeOwner`/`renounceOwnership` mutate only the owner slot; a
  `planned_updater` stored by the prior owner stays authorized to install approved
  target/hash pairs → owner rotation doesn't revoke delegated upgrade capability.
- **Remediation:** Clear all planned-upgrade authority + plan state atomically on
  owner transfer/renunciation; emit a cancellation event.

#### FLU-1051 — Validate ABI collection lengths before unmetered runtime-upgrade allocation

- **Severity:** High · **Status:** Fixed · **Linear:** `FLU-1051` · **Fix:** —
- **Where:** `crates/codec/src/vec.rs`, `crates/sdk-derive/derive-core/src/router.rs`,
  `contracts/runtime-upgrade/src/lib.rs`.
- **Impact:** The router decodes both dynamic arrays before the `planUpgrade` owner
  check; `Vec<T>::decode` calls `Vec::with_capacity(count)` before proving any body
  exists, and runtime-upgrade runs without memory-growth fuel → a tiny
  unauthenticated call forces a large reserve/zero-fill.
- **Remediation:** Prove count + head/body tables fit the buffer with checked
  arithmetic before reserve/iterate; protocol element/byte cap; `try_reserve` after
  validation; meter decode work.

### Medium

#### FLU-1052 — Charge dynamic SSTORE and LOG gas before committing Universal Token effects

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-1052` · **Fix:** —
- **Where:** `contracts/universal-token/src/lib.rs`, `crates/sdk/src/system.rs`,
  `crates/revm/src/executor.rs`.
- **Impact:** REVM precharges SLOAD-like access then commits returned `sstore`
  transitions and logs without transition-dependent SSTORE cost/refunds or LOG cost
  → token calls persist state/emit logs below canonical EVM cost. (Closely related to
  the later High `FLU-1160` in the 2026-08-20 audit.)
- **Remediation:** Charge canonical dynamic SSTORE + cold/warm + refund + LOG gas
  before committing; fail atomically on insufficient gas.

#### FLU-1053 — Parse strict WebAuthn clientDataJSON instead of matching caller-selected substrings

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-1053` · **Fix:** —
- **Where:** `contracts/webauthn/src/lib.rs`, `.../webauthn.rs`.
- **Impact:** `verifyStrict` compares caller-selected substrings without parsing
  JSON; duplicate/decoy `origin`/`type`/`challenge` members can satisfy selected
  bytes with different semantics.
- **Remediation:** Strict deterministic parse; exactly one correctly-typed member;
  reject duplicates/malformed; compare decoded values.

#### FLU-1054 — Truncated Compact u64/i64 values panic during decode

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-1054` · **Fix:** —
- **Where:** `crates/codec/src/primitive.rs`.
- **Impact:** Guard checks `ALIGN == 4` rather than decoded width; a 4–7-byte
  `u64`/`i64` passes and reaches an 8-byte read → bounds panic.
- **Remediation:** Validate `offset.checked_add(word_size)` against the chunk.

#### FLU-1055 — ABI decoders panic on out-of-range dynamic offsets and bodies

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-1055` · **Fix:** —
- **Where:** `crates/codec/src/bytes_codec.rs`, `crates/codec-derive/src/lib.rs`,
  `crates/codec/src/hash.rs`.
- **Impact:** `SolidityABI::decode` slices attacker-controlled ranges without
  validation → bounds panic instead of a codec error; WebAuthn's derived dynamic
  structs give unprivileged reachability. (Re-validated as still reproducible in the
  2026-08-20 audit.)
- **Remediation:** One checked-range helper (`checked_add`) used by all generated
  decoders.

#### FLU-1056 — Zero-length EXTCODECOPY and CALL outputs fail on ignored large offsets

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-1056` · **Fix:** —
- **Where:** `crates/evm/src/opcodes.rs`.
- **Impact:** With `len == 0` and a huge offset, the resume path narrows the offset
  and raises `InvalidOperandOOG`, changing canonical semantics for EXTCODECOPY and
  CALL/CALLCODE/DELEGATECALL/STATICCALL output ranges.
- **Remediation:** Decode length first; zero length → empty sentinel range without
  converting the offset.

#### FLU-1057 — Delegated EVM returns zero for BLOBBASEFEE and every BLOBHASH

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-1057` · **Fix:** —
- **Where:** `crates/evm/src/host.rs`, `crates/revm/src/executor.rs`,
  `crates/sdk/src/types/context.rs`.
- **Impact:** `blob_gasprice()`/`blob_hash(i)` always return zero and the shared
  context omits both → contracts commit state inconsistent with canonical EVM.
- **Remediation:** Version the shared context to carry blob gas price + ordered
  versioned hashes.

#### FLU-1058 — Genesis downloads/caches allow unbounded compressed-body buffering

- **Severity:** Medium · **Status:** Canceled · **Linear:** `FLU-1058` · **Fix:** —
- **Where:** `crates/node/src/utils.rs`, `bins/runtime-upgrade/main.rs`.
- **Impact:** A compromised response / replaced cache is fully buffered by
  `Response::bytes()` then duplicated by file reads; only decompressed JSON is capped
  → memory/disk exhaustion before authentication.
- **Remediation:** Streaming hard-limit remediation filed; ticket canceled under this
  ID.

#### FLU-1059 — rWasm Ed25519 addition returns the unchanged first operand

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-1059` · **Fix:** —
- **Where:** `crates/types/src/rwasm_context.rs`,
  `crates/runtime/src/syscall_handler/edwards/edwards_add.rs`.
- **Impact:** `ed25519_add(p, q)` returns `p` while the host writes `p + q` through
  `q_ptr` → rWasm caller gets unchanged `p`, native backend the sum
  (backend-dependent behavior on a public crypto syscall).
- **Remediation:** Write the result to `p_ptr` (or change both ABI sides
  consistently).

#### FLU-1060 — Delegated EVM always executes Osaka regardless of active fork

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-1060` · **Fix:** —
- **Where:** `crates/evm/src/evm.rs`, `.../utils.rs`, `contracts/evm/src/lib.rs`,
  `crates/node/src/chainspec.rs`.
- **Impact:** Delegated bytecode gets no active fork; `EthVM::new` always selects
  Osaka, so pre-activation opcodes (e.g. `CLZ`) execute while the spec declares
  Prague.
- **Remediation:** Carry a versioned active `SpecId` through the shared context; init
  runtime flags + gas from it; include fork in cache identity.

#### FLU-1061 — Unknown RPC transaction type stops consensus block ingestion

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-1061` · **Fix:** —
- **Where:** `crates/node/src/launcher.rs`.
- **Impact:** An unknown transaction envelope reaches `expect` and panics the
  detached subscription task; the outer loop ends "successfully" and the node
  silently stops following blocks until restart. (Still the canonical tracker as of
  the 2026-08-20 audit.)
- **Remediation:** Concrete network type or fallible conversion; supervise the
  subscription; treat unexpected termination as retryable/fatal, not success.

#### FLU-1062 — Malformed system precompiles halt without consuming supplied gas

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-1062` · **Fix:** —
- **Where:** `contracts/blake2f` (representative), `bls12381`, `kzg`, `modexp`,
  `ripemd160`, BN256 invalid-input paths; `crates/runtime/src/executor.rs`,
  `crates/revm/src/executor.rs`.
- **Impact:** A direct malformed precompile call returns `Err` before success-only
  `sync_evm_gas`; the rWasm engine reports ~no fuel and `process_halt` returns
  remaining frame gas, whereas native REVM spends all gas.
- **Remediation:** Centrally enforce that exceptional rWasm precompile halts spend
  the frame's remaining regular gas.

#### FLU-1063 — Published struct ABI selectors diverge from compiled routers

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-1063` · **Fix:** —
- **Where:** `crates/sdk-derive/derive-core/src/abi/types/conversion/rust_to_sol.rs`,
  `.../method.rs`, `crates/build/src/generators/solidity.rs`, `.../struct_parser.rs`.
- **Impact:** An unresolved struct becomes an empty tuple and fixes the router
  selector from `()`; artifact generation inserts real components only into the
  published ABI → tooling calls a selector the router won't accept.
- **Remediation:** Resolve all components before selector calculation; fail if the
  ABI-recomputed selector differs from the router selector.

#### FLU-1064 — Duplicate struct names make ABI artifacts order-dependent

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-1064` · **Fix:** —
- **Where:** `crates/build/src/generators/struct_parser.rs`.
- **Impact:** Unsorted directory results folded with `HashMap::extend` and bare-name
  keys → two modules defining `Config` overwrite by filesystem order; identical
  source yields different ABI layouts.
- **Remediation:** Deterministic module traversal; module-qualified type paths; fail
  on ambiguous names.

#### FLU-1065 — Contract builds execute mutable Docker tags without digest enforcement

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-1065` · **Fix:** —
- **Where:** `crates/build/src/lib.rs`, `crates/build/src/docker.rs`.
- **Impact:** The reproducible-build path selects a mutable `name:version` tag,
  trusts any matching local image, validates no digest, and mounts source read-write
  → a poisoned/retagged image can alter artifacts and persist in cache.
- **Remediation:** Require `name@sha256:digest` + verified provenance; compare the
  resolved digest before `docker run`; isolate writable outputs from source.

#### FLU-1066 — Signed and fixed-bytes mapping keys use noncanonical Solidity slots

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-1066` · **Fix:** —
- **Where:** `crates/sdk/src/storage/map.rs`, `.../primitive.rs`.
- **Impact:** `StorageMap::entry` right-aligns every key in a zero word; Solidity
  sign-extends negative integer keys and left-aligns/right-pads `bytesN` → the same
  logical key addresses different storage at mixed-language / proof / upgrade
  boundaries.
- **Remediation:** A mapping-key encoding trait with Solidity-compatible per-type
  rules.

#### FLU-1067 — StorageVec index multiplication wraps into earlier elements

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-1067` · **Fix:** —
- **Where:** `crates/sdk/src/storage/vec.rs`, `contracts/Cargo.toml`.
- **Impact:** For multi-slot `T`, `index * T::SLOTS as u64` wraps before conversion
  to `U256` in release; a large index via the unchecked `at` accessor aliases a low
  element and corrupts ownership/accounting state.
- **Remediation:** Multiply after converting to `U256` (or `checked_mul`); add a
  bounds-checked accessor.

#### FLU-1068 — Overlong token metadata can persist invalid UTF-8 and poison reads

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-1068` · **Fix:** —
- **Where:** `crates/sdk/src/types/storage.rs`, `contracts/universal-token/src/lib.rs`,
  `.../erc2612.rs`.
- **Impact:** Release omits the only length assertion; an overlong name/symbol is
  truncated to 32 bytes, possibly splitting a UTF-8 code point; later metadata/permit
  reads unwrap UTF-8 decoding and panic, persistently disabling those paths.
- **Remediation:** Reject overlong metadata before storage mutation; validate
  constructor lengths; return errors instead of unwrapping.

#### FLU-1069 — Solidity interface file changes do not invalidate incremental builds

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-1069` · **Fix:** —
- **Where:** `crates/sdk-derive/src/lib.rs`.
- **Impact:** Path-form Solidity macros read `.sol` files but emit no tracked
  dependency → changing only the interface can leave the crate fresh, embedding stale
  methods/selectors.
- **Remediation:** Emit a hidden `include_str!`/dep-info input for the resolved path.

#### FLU-1070 — Dynamic event data includes a noncanonical outer tuple offset

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-1070` · **Fix:** —
- **Where:** `crates/sdk-derive/derive-core/src/event.rs`,
  `contracts/runtime-upgrade/src/lib.rs`.
- **Impact:** Non-indexed event fields encoded as one dynamic tuple add an outer
  offset absent from Solidity event data → runtime-upgrade logs can be
  rejected/misread by standard decoders.
- **Remediation:** Encode non-indexed fields with top-level function-argument
  semantics.

#### FLU-1071 — Indexed reference event topics use ordinary ABI encoding

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-1071` · **Fix:** —
- **Where:** `crates/sdk-derive/derive-core/src/event.rs`.
- **Impact:** Generated topics hash ordinary ABI encoding for dynamic indexed fields
  and copy a first word for static composites, unlike Solidity's special indexed
  encoding → filters/indexers miss affected logs.
- **Remediation:** Classify value vs reference/composite and implement Solidity's
  indexed encoding exactly.

#### FLU-1072 — Generated view and pure clients issue mutable CALLs

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-1072` · **Fix:** —
- **Where:** `crates/sdk-derive/derive-core/src/sol_input.rs`, `.../client.rs`.
- **Impact:** `view`/`pure` maps to `&self` but client generation always emits
  mutable `sdk.call` → in a non-static frame a malicious target can mutate state or
  re-enter where Solidity would enforce `STATICCALL`.
- **Remediation:** Preserve mutability; use `static_call` for view/pure; forbid
  attached value.

#### FLU-1073 — Fixed byte-array selectors disagree with generated calldata codecs

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-1073` · **Fix:** —
- **Where:** `crates/sdk-derive/derive-core/src/abi/types/conversion/rust_to_sol.rs`,
  `.../codec.rs`.
- **Impact:** `[u8; N]` (N ≤ 32) canonicalizes to `bytesN` for the selector but the
  codec encodes `uint8[N]` → canonical calldata selects the route but fails decode.
- **Remediation:** Map consistently to `uint8[N]` or use a real `FixedBytes<N>` codec
  when the selector advertises `bytesN`.

#### FLU-1074 — Solidity imports silently drop unsupported functions and parameters

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-1074` · **Fix:** —
- **Where:** `crates/sdk-derive/derive-core/src/sol_input.rs`.
- **Impact:** Function/parameter conversion failures discarded with `.ok()` → a
  valid-but-unsupported type can silently remove a security parameter, change a
  selector, or drop a method while compilation succeeds.
- **Remediation:** Collect/propagate conversion errors with spans; never emit a
  partial surface.

#### FLU-1075 — Enforce WASM and rWasm size caps in runtime upgrades

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-1075` · **Fix:** —
- **Where:** `contracts/runtime-upgrade/src/lib.rs`, `crates/revm/src/syscall.rs`,
  `crates/types/src/lib.rs`.
- **Impact:** `compile_and_install` checks only the magic; neither boundary rejects
  raw WASM above 1 MiB or serialized rWasm above 12 MiB → oversized artifacts consume
  control-plane compilation resources and persist oversized code.
- **Remediation:** Reject over-limit raw input before compilation and over-limit
  serialized output before native execution; repeat the cap in the host syscall.

#### FLU-1076 — Emit the installed code hash for canonical precompile upgrades

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-1076` · **Fix:** —
- **Where:** `contracts/runtime-upgrade/src/lib.rs`, `crates/revm/src/syscall.rs`.
- **Impact:** Installers obtain `RuntimeUpgraded.codeHash` via the EVM-facing syscall,
  which returns zero for canonical precompile addresses → upgrades of
  consensus-critical crypto precompiles emit a zero artifact hash, breaking
  event-level release verification.
- **Remediation:** Compute the event value from the installed bytes (or a privileged
  raw-state code-hash syscall); keep ordinary EVM masking zero.

#### FLU-1077 — Reject trailing legacy UST constructor bytes before persisting metadata

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-1077` · **Fix:** —
- **Where:** `crates/sdk/src/universal_token/storage.rs`,
  `contracts/universal-token/src/lib.rs`, `crates/revm/src/executor.rs`.
- **Impact:** A UST payload longer than canonical V1/V2 routes to a legacy decoder
  that doesn't require end-of-input; the constructor persists the full attacker input
  (including trailing bytes) as account metadata, uncharged → underpriced state
  growth + recurring overhead.
- **Remediation:** Require exact legacy consumption; persist a canonical re-encoding;
  enforce size + deposit gas on the final serialized code.

#### FLU-1078 — Prevent zero-address ownership changes from restoring fee bootstrap authority

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-1078` · **Fix:** —
- **Where:** `contracts/fee-manager/src/lib.rs`, `crates/types/src/genesis.rs`.
- **Impact:** `changeOwner(address(0))` stores zero while `owner()`/`only_owner()`
  interpret zero as `DEFAULT_FEE_MANAGER_AUTH` → after migrating to a multisig, an
  erroneous zero transition silently reactivates the retired genesis key.
- **Remediation:** Reject `Address::ZERO` before storage mutation; keep
  `renounceOwnership()` writing `SYSTEM_ADDRESS` as the explicit fork-only transition.

#### FLU-1079 — Charge the EIP-7951 P256 verification gas schedule

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-1079` · **Fix:** —
- **Where:** `contracts/eip7951/src/lib.rs`; pinned `revm-rwasm .../secp256r1.rs`;
  `crates/genesis/build.rs`, `crates/types/src/genesis.rs`.
- **Impact:** The genesis precompile at `0x100` charges the old RIP-7212 3,450-gas
  variant although EIP-7951 specifies 6,900, with no compensating meter → attackers
  buy ~2× the intended P256 work per block.
- **Remediation:** Use `P256VERIFY_BASE_GAS_FEE_OSAKA` + `p256_verify_osaka`
  atomically; preserve invalid-input behavior.

## Notes

Cross-cutting themes: **codec/ABI canonicalization** (FLU-1055, 1063, 1064, 1066,
1070, 1071, 1072, 1073, 1074); **gas/metering parity with canonical REVM**
(FLU-1052, 1056, 1057, 1060, 1062, 1075, 1076, 1079); **unmetered/unbounded
allocation** (FLU-1046, 1047, 1051, 1054, 1058, 1077); **runtime-upgrade authority
& provenance** (FLU-1048, 1049, 1050, 1075, 1076).

## Re-review checklist for future audits

- Re-run codec/ABI parity tests against an independent Solidity encoder for
  selectors, indexed event topics, event data framing, mapping-key slots, and
  `bytesN` codecs.
- Confirm delegated-EVM opcodes read the active fork and correct blob/gas values, and
  that system-runtime + precompile halts match pinned REVM gas.
- Confirm every host decode / router argument uses checked length validation +
  `try_reserve` before allocation.
- Confirm runtime-upgrade artifacts are authenticated, size-capped, and that
  ownership transitions revoke planned-upgrade authority.
