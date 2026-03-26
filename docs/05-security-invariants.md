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

Release profile is `panic = "abort"`.
Do not rely on unwind recovery for consensus-critical paths.

Why it matters: abort behavior must be anticipated in error-handling design.

---

## 10) Review checklist for syscall-handler changes

Before merge:

- [ ] strict input/state validation preserved
- [ ] static-call checks preserved for mutating branches
- [ ] gas/fuel charging order remains deterministic
- [ ] allocation safety is bounded and prevalidated
- [ ] ownership checks remain intact
- [ ] interruption/resume symmetry still holds
