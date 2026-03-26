# Interruption Protocol (`exec` / `resume`)

## Mental model

Runtime execution is not a single uninterrupted run.
When runtime needs host-only capability, it yields control, host executes the requested action, then runtime resumes.

Think of it as a deterministic handshake:

- **exec**: runtime starts/continues and may request interruption,
- **host action**: REVM handles the request,
- **resume**: runtime continues from saved state.

---

## Signals and meaning

Runtime exit code is overloaded by protocol:

- `exit_code <= 0`: final result (success/revert/error classes)
- `exit_code > 0`: interruption request, value is `call_id`

Return bytes carry encoded syscall invocation parameters for interruption path.

---

## Interruption payload

Core payload includes:

- `call_id`: resumable runtime handle,
- syscall parameters (id/input range/fuel/state),
- gas snapshot for deterministic settlement.

This payload is decoded by host side and routed to syscall handler logic.

---

## Host-side interruption handling

For each interruption:

1. read syscall input from runtime memory,
2. validate input length/state constraints,
3. charge gas according to syscall semantics,
4. execute host operation,
5. either return immediate result or create a new frame,
6. store interruption outcome for resume phase.

This is where privileged/stateful operations happen.

---

## Resume path

After host action/frame completes, runtime is resumed with:

- `call_id`,
- mapped exit code from host action,
- returned data,
- consumed/refunded fuel,
- optional pointer for writing fuel accounting tuple.

Runtime continues from the saved execution point associated with that `call_id`.

---

## System-runtime envelopes

System runtimes use structured envelopes for interruption and final output so REVM can deterministically apply:

- return data,
- storage diff,
- logs,
- metadata updates.

Without this envelope contract, REVM would not know how to safely commit system-runtime side effects.

---

## Lifecycle guarantees

- `call_id` is transaction-scoped.
- Resumable contexts are forgotten when execution finalizes.
- Per-transaction reset clears runtime recovery state/counters.
- Resume is root-only in current runtime flow (nested user code cannot legally drive resume directly).

Protocol correctness depends on keeping these guarantees intact.
