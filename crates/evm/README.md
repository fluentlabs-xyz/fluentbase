# fluentbase-evm

A drop‑in, bytecode‑compatible Ethereum Virtual Machine (EVM) interpreter with an interruptible execution model. It runs
standard EVM bytecode and preserves semantic and gas compatibility while allowing certain host‑dependent operations (
state/account I/O, calls, logs, creates, etc.) to be executed via explicit interruptions to the surrounding runtime.
This design lets us embed the EVM inside alternative runtimes (e.g., WASM systems) without sacrificing EVM behavior,
accounting, or EIP compliance.

> TL;DR
> - Fully EVM‑compatible: same bytecode, same semantics, same gas model and refunds, Prague spec by default.
> - Interruptible: host operations are performed through well‑defined interpreter interruptions and resumed seamlessly.
> - No new opcodes: we reuse the standard opcode set; selected opcodes are routed through the interruption layer.
> - Deterministic gas: gas is synchronized with the host using a stable conversion factor (FUEL_DENOM_RATE) so total
    cost matches EVM rules.

## Why an interruptible EVM?

Traditional EVM interpreters assume direct access to a native execution environment that provides account/state access,
calls, and logging. When embedding the EVM inside a different host (for example, a WASM runtime or a modular rollup
environment), these capabilities must be delegated. An interruptible VM makes this safe and explicit:

- The interpreter pauses at specific points (opcodes that need host interaction), emits an interruption containing all
  the context it needs, and yields control to the host.
- The host executes the requested operation (e.g., CALL, SLOAD, CREATE), returns output and gas deltas, and the
  interpreter resumes exactly where it paused, applying results and continuing as if the opcode executed natively.

This yields clean separation of concerns: the interpreter focuses on EVM semantics and gas, while the host provides
environment‑specific syscalls.

## High‑level architecture

This crate builds on the excellent revm components, adapting them to an interruptible design:

- revm‑interpreter provides the core instruction dispatch and gas machinery.
- We define an EthVM wrapper that holds an `Interpreter` configured with our `InterruptionExtension` state.
- We generate an instruction table identical to EVM, but for selected opcodes we replace their handlers with
  interrupt‑aware shims.
- When such an opcode is encountered, we construct an `InterpreterAction::SystemInterruption` that carries the syscall
  payload to the host, then we jump back to the same PC so the opcode can be re‑executed after the host responds (with
  the outcome stored in the extension state).

Key types and modules:

- EthVM (src/evm.rs): top‑level driver with `run_the_loop` that executes until Return. It integrates with a `SharedAPI`
  host via HostWrapperImpl.
- `InterruptionExtension` and `InterruptionOutcome` (src/types.rs): per‑interpreter extension that stores the last
  interruption result and tracks committed_gas.
- `interruptable_instruction_table` (src/opcodes.rs): builds the instruction table and swaps handlers for host‑bound
  opcodes.
- utils.rs: helper utilities: `interrupt_into_action` (creates a `SystemInterruption`), `sync_evm_gas` (keeps host and
  interpreter gas in lock‑step), and memory helpers.
- bytecode.rs: `AnalyzedBytecode`, including jump table analysis and compact serialization.
- host.rs: `HostWrapperImpl` bridges the host `SharedAPI` to revm’s Host trait.

By default the interpreter uses `SpecId::PRAGUE`.

## What does “interruptible” mean technically?

- Selected opcodes are executed in two phases:
    1) Before host call: the handler prepares inputs, synchronizes gas with the host, and requests an interruption using
       `interrupt_into_action`. The interpreter yields with `InterpreterAction::SystemInterruption`.
    2) After host call: the external host executes the requested operation and returns output, exit code, and fuel
       usage. EthVM captures this in `InterruptionOutcome`, and the opcode handler then resumes, reading the stored
       outcome and completing as if everything ran in one go.

- Gas synchronization: we maintain two counters — the interpreter’s gas and the committed gas charged at the host level.
  Before any host call, we commit the delta using sync_evm_gas, converting EVM gas to host fuel via `FUEL_DENOM_RATE`.
  After a host operation returns, we merge the host‑reported fuel usage back into EVM gas with the same conversion,
  preserving EVM’s gas semantics and refunds.

- Re‑execution safety: we set a relative jump of −1 on interruption so that after the host response is installed, the
  same opcode PC is dispatched again, this time observing an available `InterruptionOutcome` and completing
  deterministically.

## Which opcodes are routed through interruptions?

We keep the full EVM opcode set and semantics. For opcodes that inherently need host interaction, we replace the
internal handler with a shim that performs the interruption step. The table is built in
`interruptable_instruction_table` and currently covers (non‑exhaustive):

- Environment/account access: BALANCE, EXTCODESIZE, EXTCODECOPY, EXTCODEHASH, BLOCKHASH, SELFBALANCE
- State access: SLOAD, SSTORE, TLOAD, TSTORE
- Logs: LOG0..LOG4
- Contract lifecycle: CREATE, CREATE2, SELFDESTRUCT
- Calls: CALL, CALLCODE, DELEGATECALL, STATICCALL

Each shim mirrors the canonical EVM behavior: identical stack/memory I/O, identical gas charges/refunds, and identical
success/failure rules. We do not add new opcodes and do not change encodings or stack conventions.

## Compatibility and EIP compliance

- Bytecode compatibility: Any valid Ethereum bytecode is accepted. We analyze and execute legacy bytecode using the same
  jump table and control‑flow rules as upstream revm.
- Spec conformance: The interpreter is configured for `SpecId::PRAGUE` and follows the associated EVM rules. The opcode
  set, gas costs, and semantics align with mainnet EIPs active for Prague unless the embedding runtime purposefully
  configures otherwise.
- EIP coverage: Because we delegate to revm components and keep identical opcode semantics, the engine complies with the
  relevant EVM EIPs (e.g., CREATE2, SSTORE gas schedule changes across forks, log topics, staticcall restrictions,
  etc.).
- No semantic changes: Interruptibility only changes how host interactions are performed, not what they do. Programs
  observe the same results and gas as on a standard EVM.

Note: If you embed this VM into an environment with different limits (e.g., maximum code size, call depth, or disabled
blobs), ensure your host enforces or mirrors the network rules you target. This crate focuses on the interpreter and its
host boundary, not on network rules enforcement.

## Gas and fuel model

- The interpreter tracks EVM Gas as usual.
- The host may operate in a different “fuel” unit; we use a constant `FUEL_DENOM_RATE` to convert between units.
- Before host calls, we “commit” the difference between the interpreter’s gas state and the last committed snapshot by
  calling `sdk.charge_fuel_manually(remaining_diff * FUEL_DENOM_RATE, refund_diff * FUEL_DENOM_RATE)`.
- After a host operation completes, we convert the host fuel usage back into an EVM Gas and set it as the current gas so
  the loop remains consistent.
- ExecutionResult exposes `chargeable_fuel_and_refund` so the embedding layer can settle cost precisely.

This ensures total cost and refunds match standard EVM behavior when seen from within the VM.

## Using the crate

1) Analyze/commit bytecode and store alongside its code hash. For example, from `contracts/evm/lib.rs`:

```rust
use fluentbase_evm::{bytecode::AnalyzedBytecode, EthVM};
use fluentbase_sdk::{ContextReader, SharedAPI, Bytes};

// Persist code and hash (host‑specific; example from this repo)
pub(crate) fn commit_evm_bytecode<SDK: SharedAPI>(sdk: &mut SDK, evm_bytecode: Bytes) {
    let contract_address = sdk.context().contract_address();
    let evm_code_hash = fluentbase_fluentbase_sdk::crypto::crypto_keccak256(evm_bytecode.as_ref());
    sdk.metadata_write(&contract_address, 0, evm_code_hash.into()).unwrap();
    sdk.metadata_write(&contract_address, 32, evm_bytecode.into()).unwrap();
}

// Load analyzed bytecode from metadata
pub(crate) fn load_evm_bytecode<SDK: SharedAPI>(sdk: &SDK) -> Option<AnalyzedBytecode> {
    let bytecode_address = sdk.context().contract_bytecode_address();
    let (metadata_size, _, _, _) = sdk.metadata_size(&bytecode_address).unwrap();
    if metadata_size == 0 { return None; }
    let mut metadata = sdk.metadata_copy(&bytecode_address, 0, metadata_size).unwrap();
    let evm_code_hash = fluentbase_sdk::B256::from_slice(&metadata[0..32]);
    if evm_code_hash == fluentbase_sdk::B256::ZERO || evm_code_hash == fluentbase_sdk::KECCAK_EMPTY { return None; }
    fluentbase_sdk::bytes::Buf::advance(&mut metadata, 32);
    Some(AnalyzedBytecode::new(metadata, evm_code_hash.into()))
}
```

2) Execute with EthVM:

```rust
use fluentbase_evm::{EthVM, ExecutionResult};
use fluentbase_sdk::{SharedAPI, ContextReader, Bytes};

pub fn run<SDK: SharedAPI>(sdk: &mut SDK, input: Bytes) -> ExecutionResult {
    let analyzed = load_evm_bytecode(sdk).expect("no bytecode");
    let vm = EthVM::new(sdk.context(), input, analyzed);
    vm.run_the_loop(sdk)
}
```

The VM will run until completion. When an opcode needs host interaction, the loop yields a `SystemInterruption`. The
host executes the syscall (via `SharedAPI`), and the VM resumes transparently. On exit, you receive an `ExecutionResult`
with precise gas/fuel accounting.

## Design details and key functions

- `EthVM::run_the_loop` (src/evm.rs): the main loop. It feeds our instruction table to the interpreter and handles
  `SystemInterruption` by invoking the host, converting fuel <-> gas with `FUEL_DENOM_RATE`, and installing
  `InterruptionOutcome` before continuing.
- `interruptable_instruction_table` (src/opcodes.rs): constructs the instruction table and replaces handlers for
  host‑bound opcodes.
- `utils::interrupt_into_action`: packages the interruption details (code hash, input, fuel limit, and serialized state)
  and installs `InterpreterAction::SystemInterruption`; also sets a relative jump of −1 to re‑dispatch the opcode after
  the host returns.
- `utils::sync_evm_gas`: synchronizes interpreter gas with the host’s accounting before any interruption.
- `types::ExecutionResult` and `InterruptionOutcome`: encode results and conversions in a reusable way.

## Guarantees

- Semantic parity: For supported forks, behavior and gas match EVM rules and EIPs.
- Determinism: Re‑execution after interruption yields identical results to uninterrupted execution.
- Extensibility: New host syscalls can be wired in by adding or swapping opcode shims without touching the EVM core.

## Limitations and notes

- Network rules: The crate implements interpreter semantics. It assumes the embedding layer enforces chain‑level
  constraints (e.g., code size limits, blob availability, precompile sets) matching your target network.
- Precompiles: These can be modeled either as native host calls reachable via code hash routing, or embedded as regular
  opcodes depending on your environment. Ensure their gas and failure modes match the spec.
- Spec version: Defaults to SpecId::PRAGUE; configure your environment consistently with your target network.

## Versioning and stability

This crate evolves with upstream revm and EVM specifications. Breaking changes are avoided in public APIs where
possible, but low‑level details of the interruption protocol may evolve to better integrate with hosts. We maintain
bytecode compatibility and EVM semantics throughout.