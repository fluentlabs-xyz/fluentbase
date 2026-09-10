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

Syscall fuel procedures (`crates/types/src/block_fuel.rs`) address the metered length parameter by
its position among the import's parameters. rWasm 0.4.x read that position as a raw 32-bit stack
depth; the fuel-alignment change that follows 0.5.0 counts it from the last parameter (`1` is the
last one) and rejects out-of-range positions at compile time. Every metered Fluentbase syscall takes
only `i32` parameters, so both rules resolve to the same slot and the table is valid under either.

---

## Execution contract

Fluentbase runtime executor owns:

- execution dispatch (contract mode vs system mode),
- call-id based recoverable contexts,
- `execute/resume/memory_read` bridge used by REVM interruption handler.

This is the concrete runtime-host handshake point used in every interruption cycle.

System runtimes are instantiated on Wasmtime through `WasmtimeExecutor::try_new`, so a hint that
fails to link or instantiate is reported as an `IllegalOpcode` admission error instead of panicking
the node.

Structured system-runtime outcomes are decoded completely before the host applies their effects.
Collection counts must fit the remaining encoded body before reservation or iteration; byte payloads
use checked zero-copy slices. These input-derived bounds also apply with bincode's legacy configuration,
so safety does not depend on a fixed envelope-size cap. Generic log readers without lookahead grow
their buffers only after reading the corresponding data. Existing envelope encodings, including
outcomes without the optional trailing touched-slot or transfer fields, remain supported.

An invalid envelope after a successful runtime exit halts the frame with `MalformedBuiltinParams`
and clears the raw envelope from return data. If execution already failed (for example, out of fuel),
the host preserves that failure reason. No buffered effects or preload refunds are applied on decode
failure; the existing frame rollback path handles any earlier execution effects.

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
   `process_execution_result`, `process_runtime_execution_outcome`;
   `crates/runtime/src/executor.rs`: `resume`,
   `memory_read`) — an upgrade that changes trap/interruption behavior may make these
   deterministic `UnknownError` halts reachable, and they must stay typed deterministic halts or
   block-execution errors, not panics. The default release profile unwinds while the reproducible
   profile aborts, so consensus behavior cannot rely on either panic strategy,
6. update docs in same PR.

If one of these steps is skipped, regressions can escape into consensus path.

## Curve dependency upgrades

`sp1-curves` is pinned to `=5.2.4`. The raw Weierstrass add/double syscalls validate
coordinate bounds, but do not require curve membership. Version 5.2.4 uses generic
field arithmetic for secp256k1; version 6.1.0 switches to `k256` point conversion and
unwraps the result in `sw_add_k256` and `sw_double_k256`. Reduced off-curve coordinates
that reach these syscalls can therefore panic the host after an unchecked upgrade.

Before changing the pin:

- Inspect the resolved implementation of curve addition, doubling and decompression
  for panics on off-curve points, infinity, equal points and unreduced coordinates.
- Run the runtime Weierstrass tests in release mode with both `std` and `std,wasmtime`.
  Keep `secp256k1_off_curve_inputs_preserve_arithmetic_without_panicking` passing;
  it covers reduced off-curve inputs that coordinate-bound checks permit today.
- Patch an incompatible dependency before adopting it. Rejecting previously accepted
  inputs is a protocol behavior change and requires explicit compatibility review,
  including proof/runtime agreement; a dependency bump must not silently introduce it.
- Recheck all workspace lockfiles and record the version and regression results in
  the upgrade PR.
