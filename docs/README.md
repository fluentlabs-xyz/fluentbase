# Fluentbase Core Docs

This folder explains how Fluentbase execution actually works: routing, runtime boundaries, interruption flow, gas/fuel accounting, and upgrade controls.

The goal is practical understanding for contributors and auditors, not API marketing.

## Reading order

1. [`00-system-overview.md`](./00-system-overview.md) — end-to-end picture of execution.
2. [`01-runtime-routing-and-ownable-accounts.md`](./01-runtime-routing-and-ownable-accounts.md) — why contracts are wrapped and how runtime owner routing works.
3. [`02-interruption-protocol.md`](./02-interruption-protocol.md) — `exec/resume` handshake and call-id lifecycle.
4. [`03-syscall-reference-core.md`](./03-syscall-reference-core.md) — syscall layers and behavioral rules.
5. [`04-gas-and-fuel.md`](./04-gas-and-fuel.md) — gas/fuel conversion and metering model.
6. [`05-security-invariants.md`](./05-security-invariants.md) — invariants that must stay true.
7. [`06-runtime-upgrade.md`](./06-runtime-upgrade.md) — runtime-upgrade governance and host enforcement.
8. [`07-rwasm-integration.md`](./07-rwasm-integration.md) — Fluentbase integration contract with rWasm.
9. [`08-universal-token.md`](./08-universal-token.md) — UST20 runtime behavior and constraints.

## Source-of-truth rule

- Code is authoritative.
- If behavior changes in runtime-critical areas, docs must be updated in the same PR.
