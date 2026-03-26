# System Overview

## Main execution crates

- `crates/revm`: EVM handler integration + journal/state application + syscall interruption handling.
- `crates/runtime`: rWasm runtime executor (contract mode + system mode), resumable contexts.
- `crates/evm`: interruptible EVM interpreter used by `contracts/evm` runtime.
- `crates/sdk`: contract-facing APIs and system runtime context implementation.
- `crates/types`: shared constants, syscall indexes, address map, runtime wire structs.

## Two runtime modes in `crates/runtime`

Implemented by `ExecutionMode` (`crates/runtime/src/runtime.rs`):

1. **Contract runtime** (`ContractRuntime`)
   - Uses rWasm strategy executor.
   - Used for untrusted user contracts.
2. **System runtime** (`SystemRuntime`)
   - Wasmtime-backed cached executors.
   - Used for selected precompile/system addresses.

Mode selection happens in `RuntimeFactoryExecutor::execute` (`crates/runtime/src/executor.rs`):
- if `is_execute_using_system_runtime(address)` => system mode
- else => contract mode

## End-to-end call flow

### Regular call path

1. REVM frame runs via `run_rwasm_loop` (`crates/revm/src/executor.rs`).
2. `execute_rwasm_frame` builds `SharedContextInputV1` + call input.
3. `syscall_exec_impl` invokes runtime (`crates/runtime/src/syscall_handler/host/exec.rs`).
4. Runtime returns either:
   - final exit code (`<= 0`), or
   - interruption call id (`> 0`) with encoded `SyscallInvocationParams`.
5. REVM applies result via `process_exec_result`.

### Interruption path

1. Positive exit code is treated as `call_id`.
2. REVM decodes `SyscallInvocationParams`.
3. `execute_rwasm_interruption` executes host-side action (sload/call/create/log/etc.).
4. REVM resumes runtime using `syscall_resume_impl`.
5. Result is applied to REVM frame and journal.

## System runtime structured envelopes

For system runtimes, REVM and runtime exchange structured payloads (`crates/types/src/system/*`):

- `RuntimeNewFrameInputV1`
- `RuntimeInterruptionOutcomeV1`
- `RuntimeExecutionOutcomeV1`

REVM decodes and applies these in `process_runtime_execution_outcome` (`crates/revm/src/executor.rs`):
- output bytes,
- storage writes,
- logs,
- ownable account metadata updates.

## Precompile/system address map

Defined in `crates/types/src/genesis.rs`:

- delegated runtimes: EVM/SVM/WASM/UniversalToken
- standard EVM precompiles
- governance/system contracts (runtime-upgrade, fee manager, etc.)

Key helper sets:
- `is_evm_system_precompile(...)`
- `is_execute_using_system_runtime(...)`
- `is_engine_metered_precompile(...)`
