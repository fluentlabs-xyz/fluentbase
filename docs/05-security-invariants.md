# Security Invariants

This file lists invariants that must hold to avoid consensus failures, privilege breaks, and host crashes.

## 1) Runtime routing invariants

- Created contracts are wrapped as `OwnableAccount` and routed by init-code magic (`crates/revm/src/evm.rs`, `crates/types/src/genesis.rs`).
- Direct calls targeting delegated runtime addresses are rejected in `execute_rwasm_frame`.
- Metadata mutation is only allowed for ownable accounts owned by the same runtime owner.

## 2) Interruption invariants

- Positive runtime exit code means interruption `call_id`; non-positive means final halt code.
- `call_id` contexts are transaction-scoped and must be cleared on final return/reset.
- Resume must only use recoverable contexts associated with that `call_id`.

## 3) Bounds and allocation invariants

- Guest-provided lengths must be validated before host allocation.
- Memory reads from guest/runtime memory must fail gracefully on OOB.
- `CODE_COPY` length is bounded by `EXT_CODE_COPY_MAX_COPY_SIZE`.

## 4) Static-context invariants

Mutating operations must reject static context, including:
- storage writes,
- metadata writes/creates,
- account-destroy/upgrades,
- token/runtime state mutations.

## 5) System runtime envelope invariants

For system runtime addresses only:

- output is structured (`RuntimeExecutionOutcomeV1`) and decoded by REVM,
- storage/log/metadata updates are applied only when runtime exit is `Ok`,
- fatal runtime exits must not be interpreted as structured payloads.

## 6) Authority invariants

- Runtime upgrade syscall is restricted to `PRECOMPILE_RUNTIME_UPGRADE` caller path.
- Governance owner logic is implemented in `contracts/runtime-upgrade`.
- Any change to update authority constants requires explicit security review.

## 7) Non-user-controllable fatal codes

Non-system contracts must not be able to surface internal fatal runtime-only exit classes into normal user flow.
(REVM currently remaps selected fatal codes to `UnknownError` in `process_execution_result`.)

## 8) Bridge hook invariants

Bridge hooks (`crates/revm/src/bridge.rs`) assume strict event shapes/counts and mutate bridge balance accordingly.
Any change in bridge event ABI or transaction flow requires synchronized updates in hooks.

## 9) Panic policy

`panic = "abort"` is enabled in workspace release profile.
Do not rely on unwind-based recovery.
Consensus-critical paths should prefer explicit error returns over panics.

## 10) Review checklist for touching `crates/revm/src/syscall.rs`

Before merging changes to syscall handler:

- [ ] input/state length checks are exact
- [ ] static-context checks for mutations exist
- [ ] gas charging order is deterministic
- [ ] host allocations are bounded and prevalidated
- [ ] ownership checks are preserved for metadata/account ops
- [ ] interruption outcome path remains symmetric (`exec`/`resume`)
