# rWasm Integration Notes

This document describes Fluentbase-specific integration points with `fluentlabs-xyz/rwasm`.

## Canonical rWasm docs

Use rWasm upstream docs for VM internals:
- `rwasm/docs/architecture.md`
- `rwasm/docs/pipeline.md`
- `rwasm/docs/module-format.md`
- `rwasm/docs/vm-and-fuel.md`
- `rwasm/docs/security-considerations.md`

This file only covers Fluentbase wiring.

## Import surface

Fluentbase builds `ImportLinker` in `crates/types/src/import_linker.rs`:

- module: `fluentbase_v1preview`
- function names: `_read`, `_write`, `_exec`, `_resume`, hash/bn/bls ops, etc.
- each import is assigned syscall index (`SysFuncIdx`) and fuel procedure.

## Compilation configuration used by Fluentbase

`crates/sdk/src/types/rwasm.rs`:

- state router maps:
  - `deploy` -> `STATE_DEPLOY`
  - `main` -> `STATE_MAIN`
- state opcode: `Opcode::Call(SysFuncIdx::STATE as u32)`
- for user contracts:
  - malformed entrypoint signatures disallowed
  - default memory-page limit (`N_DEFAULT_MAX_MEMORY_PAGES`)
- for system runtimes:
  - malformed entrypoint signatures allowed
  - max memory pages raised (`N_MAX_ALLOWED_MEMORY_PAGES`)
  - fuel metering mode depends on `is_engine_metered_precompile`

## Runtime executors used in Fluentbase

- `ContractRuntime`: strategy executor for untrusted contracts.
- `SystemRuntime`: Wasmtime-backed cached executors for system runtimes.

`SystemRuntime` keeps thread-local cache keyed by code hash.
Before each call it swaps `RuntimeContext` into executor store and swaps back after execution.

## Resume and memory access integration

`RuntimeFactoryExecutor` (`crates/runtime/src/executor.rs`) owns:

- recoverable runtime map by `call_id`,
- `execute(...)`, `resume(...)`, `memory_read(...)` bridging used by REVM interruption handler.

This is the boundary used by `crates/revm/src/syscall.rs` to fetch syscall params and continue execution.

## Versioning and upgrade discipline

Fluentbase depends on exact rWasm behavior for:

- syscall import ABI,
- memory read/write semantics,
- fuel accounting behavior,
- trap/interrupt behavior.

When bumping rWasm:

1. bump Cargo deps,
2. re-run e2e interruption/resume tests,
3. re-check allocation/bounds safety on memory read helpers,
4. update these docs in same PR.
