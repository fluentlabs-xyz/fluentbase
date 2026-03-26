# Runtime Routing and Ownable Accounts

## Why this model exists

Fluentbase separates **account identity** from **execution engine**.

Instead of storing one custom bytecode implementation per account for every runtime family,
accounts can be wrapped as an ownable account that points to a delegated runtime owner.

This gives:
- shared runtime logic,
- per-account isolated state,
- deterministic routing by owner/runtime type.

---

## Ownable account format

Ownable account bytecode carries:

- a magic/version header,
- `owner_address` (the delegated runtime address),
- runtime metadata bytes.

`owner_address` is the key field: it decides which runtime code executes the account.

---

## Create-time routing

On contract creation, init code is inspected by magic prefix.

Resolver outcomes:
- wasm/rwasm payload -> wasm delegated runtime
- svm ELF payload (feature-gated) -> svm delegated runtime
- universal token magic -> universal token delegated runtime
- otherwise -> delegated EVM runtime

After routing:
- new account code is set as ownable wrapper,
- original init payload is passed to delegated runtime for deploy logic.

So deployment decides runtime class, not a later toggle.

---

## Execution-time behavior

When REVM executes an ownable account:

- it loads code from `owner_address` (the delegated runtime),
- keeps current account as state owner/target,
- forwards call input to delegated runtime logic.

This is why many accounts can share one runtime implementation without sharing state.

---

## Direct-runtime-call restrictions

Delegated runtime addresses are not intended to behave like normal user contracts.
Direct execution targeting runtime-owner addresses is blocked in execution path.

Reason: user flow must go through wrapped account semantics, not bypass routing/account invariants.

---

## Metadata ownership rules

Metadata syscalls are scoped by runtime ownership.

Mutation is allowed only when:

1. target account is ownable account, and
2. target owner address matches caller runtime owner.

This prevents one runtime family from rewriting metadata of accounts owned by another runtime.

Static-context mutations are rejected for metadata-changing operations.

---

## Wasm-wrapper deploy rewrite (special path)

There is an additional deploy-stage rewrite path used by wasm runtime pipeline:

- deploy output may contain compiled rWasm payload + constructor tail,
- deployed code can be rewritten to direct rWasm bytecode,
- constructor continuation is executed with remaining params.

This exists to support wasm-wrapper deployment flow while keeping final account code/runtime semantics consistent.
