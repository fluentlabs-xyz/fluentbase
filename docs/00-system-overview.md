# System Overview

## What Fluentbase is doing at runtime

Fluentbase does not execute every contract with one uniform engine.
It routes execution through runtime owners and then coordinates state changes through REVM.

At a high level:

1. A call or create enters REVM.
2. REVM determines which runtime should execute the logic.
3. Runtime executes and may ask host for privileged/stateful actions.
4. REVM applies final output + journal updates.

The important part is that execution and state commitment are deliberately split.

---

## Main components and their jobs

- **REVM integration layer**: frame lifecycle, journal, host syscall handling.
- **Runtime executor**: runs rWasm modules in two modes (untrusted contract mode vs trusted system mode), tracks resumable contexts.
- **Interruptible EVM runtime**: used by delegated EVM runtime contract.
- **SDK/runtime context layer**: contract-facing APIs and structured envelope handling.
- **Shared types/constants**: syscall indexes, address maps, limits, fuel/gas constants, wire structs.

---

## Two execution modes

### 1) Contract mode
For regular untrusted contracts.

- isolated execution context
- strict bounds/fuel handling
- no privileged assumptions

### 2) System mode
For selected system/precompile runtimes.

- cached compiled executors
- structured output envelopes
- special handling on finalization and interruption

Mode is selected by address classification (system-runtime set vs normal contracts).

---

## Normal call lifecycle

1. REVM prepares frame input/context.
2. Runtime is invoked with input + fuel limit.
3. Runtime returns either final result or interruption id.
4. If final: REVM maps exit/output into instruction result.
5. If interruption: host action is executed and runtime is resumed.
6. Journal updates are committed by REVM rules.

---

## Why interruption exists

Some operations cannot be safely/fully done inside pure runtime execution (for example EVM journal operations, frame creation, host account/code queries).

So runtime asks host to perform the operation, then continues from the saved point.
This keeps host authority centralized while preserving deterministic runtime flow.

---

## Structured system-runtime envelopes

System runtimes use structured payloads so REVM can apply updates deterministically:

- new-frame input envelope
- interruption outcome envelope
- final execution outcome envelope

This is how output, storage diff, logs, and metadata updates are transported across the runtime boundary.

---

## Address map is part of consensus surface

Precompile/delegated-runtime/system addresses are fixed in shared constants.
Changing these mappings changes execution routing and is consensus-sensitive.

That includes:
- delegated runtime owners (EVM/SVM/WASM/Universal Token)
- governance/system contracts (runtime-upgrade, fee manager, bridge, etc.)
