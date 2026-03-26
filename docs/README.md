# Fluentbase Core Docs

This folder documents consensus-critical execution behavior.

## Reading order

1. [`00-system-overview.md`](./00-system-overview.md)
2. [`01-runtime-routing-and-ownable-accounts.md`](./01-runtime-routing-and-ownable-accounts.md)
3. [`02-interruption-protocol.md`](./02-interruption-protocol.md)
4. [`03-syscall-reference-core.md`](./03-syscall-reference-core.md)
5. [`04-gas-and-fuel.md`](./04-gas-and-fuel.md)
6. [`05-security-invariants.md`](./05-security-invariants.md)
7. [`06-runtime-upgrade.md`](./06-runtime-upgrade.md)
8. [`07-rwasm-integration.md`](./07-rwasm-integration.md)
9. [`08-universal-token.md`](./08-universal-token.md)

## Source-of-truth rule

- If this folder conflicts with code, code is authoritative.
- Update docs in the same PR as behavioral changes in:
  - `crates/revm`
  - `crates/runtime`
  - `crates/types`
  - `crates/sdk`
  - `contracts/*` system runtimes
