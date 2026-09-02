# Security Invariants

These are the invariants that keep Fluentbase safe and deterministic.
Break any of them and you risk consensus splits, privilege bugs, or host instability.

## 1) Routing integrity

- Newly created contracts must be routed to the correct delegated runtime class.
- User calls must not bypass routing by directly executing delegated runtime addresses.
- Metadata ownership boundaries between runtime families must be preserved.

Why it matters: wrong routing can execute different logic for the same state.

---

## 2) Interruption integrity

- Positive exit codes are interruption call IDs, not final statuses.
- Resume must use the exact recoverable context for that call ID.
- Recovery state must be cleared/reset per transaction lifecycle.

Why it matters: call-id confusion can corrupt execution flow or leak state across frames.

---

## 3) Bounds-before-allocation

- Untrusted lengths must be validated before host allocations.
- Memory reads/writes must fail safely on OOB.
- Large copy paths must be bounded.

Why it matters: this is the main line of defense against memory-based DoS.

---

## 4) Static-call immutability

State-changing operations must reject static context.

Applies to:
- storage mutations,
- metadata mutations,
- account lifecycle mutations,
- privileged runtime state transitions.

Why it matters: static call semantics are part of EVM compatibility and safety.

---

## 5) System-runtime envelope discipline

For system runtimes:

- structured output must decode deterministically,
- storage/log/metadata effects are committed only on successful runtime exit,
- fatal exits must not be interpreted as normal structured outcomes.

An unexpected trap means that the current system-runtime invocation cannot continue. The host must:

1. discard the runtime output without decoding or applying its structured envelope,
2. evict the trapped runtime instance so its stack and linear memory are never reused,
3. return a deterministic exceptional halt and revert the active REVM journal checkpoint, including
   value transfers and effects committed by nested frames.

Halting the frame or transaction does not continue from corrupted guest state. The runtime instance
is discarded, and journal rollback protects user storage and balances. Normal failed-transaction
effects, such as gas payment and nonce consumption, still follow the transaction rules.

A sandboxed guest trap alone is not evidence that host memory or persistent state was corrupted, so
it must not become an incidental process panic. If a particular invariant requires rejecting the
whole block rather than halting one transaction, propagate a typed block-execution error. Reserve
process termination for failures where host integrity cannot be trusted or safe rollback cannot be
established.

Why it matters: envelope mis-handling can commit invalid side effects.

---

## 6) Upgrade authority boundaries

- runtime-upgrade path must remain tightly scoped,
- authority defaults/owner transitions must be explicit and reviewed,
- governance key handling is high-risk surface.

Why it matters: upgrade authority compromise is full-system compromise.

---

## 7) Fatal-code containment

Non-system user contracts must not be able to surface internal fatal runtime-only classes as normal outputs.

Why it matters: prevents exposing internal failure classes as user-controlled behavior.

---

## 8) Bridge hook consistency

Bridge hooks rely on expected event/data shape and ordering.
Any ABI or flow change must update hook logic in sync.

Why it matters: mismatch can mint/burn/settle wrong amounts.

---

## 9) Panic policy

The default release profile uses `panic = "unwind"`; the reproducible release profile overrides it
with `panic = "abort"`. Consensus behavior must not depend on which profile built the binary.

- A panic is not a transaction or block rollback mechanism.
- Consensus-reachable failures must return deterministic frame, transaction, or block errors.
- Do not rely on catching an unwind: another production profile may abort instead.
- Do not rely on an abort for state safety: journal checkpoints and explicit database commit
  boundaries provide that safety.
- Reserve panics for genuine programmer invariants that cannot be reached from transaction, block,
  runtime, or other externally influenced input.

Why it matters: the same deterministic input must not become a node-crash or chain-liveness vector,
and panic-profile differences must not change consensus outcomes.

---

## 10) Review checklist for syscall-handler changes

Before merge:

- [ ] strict input/state validation preserved
- [ ] static-call checks preserved for mutating branches
- [ ] gas/fuel charging order remains deterministic
- [ ] allocation safety is bounded and prevalidated
- [ ] ownership checks remain intact
- [ ] interruption/resume symmetry still holds
