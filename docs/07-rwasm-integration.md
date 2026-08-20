# rWasm Integration (Fluentbase View)

This document explains what Fluentbase expects from rWasm and how the boundary is wired.
It is not a replacement for upstream rWasm architecture docs.

## Integration contract in one sentence

Fluentbase uses rWasm as execution engine, but defines its own syscall ABI, state routing, fuel policy, and interruption protocol on top.

---

## Import ABI boundary

Fluentbase publishes one import namespace (`fluentbase_v1preview`) containing runtime syscalls.
Each import is mapped to:

- a syscall index,
- a fuel procedure,
- strict parameter/result shape.

This import table is part of protocol behavior. Changing it is not a local refactor.

---

## Compilation contract

Fluentbase compilation config defines:

- deploy/main state routing,
- state selector opcode wiring,
- entrypoint strictness,
- memory-page limits,
- whether engine fuel metering is enabled.

System runtimes and user contracts intentionally compile with different constraints.

---

## Execution contract

Fluentbase runtime executor owns:

- execution dispatch (contract mode vs system mode),
- call-id based recoverable contexts,
- `execute/resume/memory_read` bridge used by REVM interruption handler.

This is the concrete runtime-host handshake point used in every interruption cycle.

---

## Why version bumps are risky

Fluentbase depends on exact rWasm behavior for:

- import ABI behavior,
- memory read/write safety semantics,
- trap/interruption behavior,
- fuel accounting details.

A dependency bump can silently change any of these.

---

## Required process on rWasm upgrade

1. bump dependency pins,
2. rerun interruption/resume e2e paths,
3. recheck allocation/bounds safety on memory helper paths,
4. verify gas/fuel settlement remains deterministic,
5. re-audit the invariant-violation halt paths on the rWASM↔REVM resume boundary
   (`crates/revm/src/executor.rs`: `execute_rwasm_resume`, `process_exec_result`,
   `process_runtime_execution_outcome`; `crates/runtime/src/executor.rs`: `resume`,
   `memory_read`) — an upgrade that changes trap/interruption behavior may make these
   deterministic `UnknownError` halts reachable, and they must stay halts, not panics
   (release profile is `panic = "abort"`),
6. update docs in same PR.

If one of these steps is skipped, regressions can escape into consensus path.
