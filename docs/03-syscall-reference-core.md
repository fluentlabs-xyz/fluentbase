# Syscall Surface (Core)

Fluentbase uses two syscall layers. They are related but not identical.

## Layer A: runtime import syscalls

This is the public import module exposed to runtime code (`fluentbase_v1preview`).
Examples:

- input/output operations,
- state selector,
- `exec`/`resume`,
- fuel APIs,
- hash/crypto builtins.

This layer is where runtime code calls into host ABI.
Fuel procedures for these imports are attached at compile/translation stage.

---

## Layer B: interruption syscall IDs

This is the host-side operation set handled by REVM interruption logic (`SYSCALL_ID_*`).

Main groups:

- storage/transient/block access,
- call/create/destroy operations,
- code/balance/account queries,
- metadata ownership and metadata storage operations,
- runtime-upgrade governance syscall.

This layer is not a generic public API; it is part of runtime-host handshake semantics.

---

## Practical rule: treat syscall behavior as protocol, not helper code

For each syscall, behavior must stay deterministic across nodes:

- input validation,
- static-context checks,
- gas/fuel charging order,
- output encoding,
- error/exit mapping,
- ownership checks.

Changing any of these is consensus-sensitive.

---

## High-impact constraints currently enforced

- mutating operations reject static context,
- metadata mutation requires same runtime owner,
- code copy is size-capped,
- runtime-upgrade syscall is restricted to upgrade precompile path,
- malformed length/state causes deterministic halt/error.

---

## Safe extension checklist

When adding or changing syscall behavior, update all of these together:

1. ID definitions,
2. host handler semantics,
3. fuel schedule/import linker entries (if import-layer change),
4. tests (unit + e2e interruption paths).

If only one side is changed, protocol drift is likely.
