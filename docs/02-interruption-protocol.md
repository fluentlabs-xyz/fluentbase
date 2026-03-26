# Interruption Protocol (EXEC/RESUME)

## Core objects

Defined in `crates/revm/src/types.rs` and `crates/types/src/syscall.rs`:

- `SystemInterruptionInputs`
  - `call_id: u32`
  - `syscall_params: SyscallInvocationParams`
  - `gas: Gas` snapshot
- `SystemInterruptionOutcome`
  - original `inputs`
  - optional host `ExecutionResult`
  - `halted_frame` flag
- `SyscallInvocationParams`
  - `code_hash`, `input` range, `fuel_limit`, `state`, `fuel16_ptr`

## Producer side (`_exec`)

1. Runtime executes contract entrypoint via `syscall_exec_impl`.
2. If runtime needs host action, it exits with positive `call_id` and encoded syscall params in return bytes.
3. REVM (`process_exec_result`) interprets `exit_code > 0` as interruption.

## Host side handling

`execute_rwasm_interruption` (`crates/revm/src/syscall.rs`) does:

1. decode guest syscall input from runtime memory (`default_runtime_executor().memory_read(...)`)
2. validate length/state constraints
3. charge REVM gas according to syscall semantics
4. perform host action:
   - immediate result, or
   - create new REVM frame for CALL/CREATE-like operations
5. store interruption outcome in frame and continue loop

## Resume side (`_resume`)

When child action/frame finishes, REVM calls `syscall_resume_impl` (`crates/runtime/src/syscall_handler/host/resume.rs`):

inputs:
- `call_id`
- returned output bytes
- mapped exit code
- consumed/refunded fuel
- optional pointer for writing `(fuel_consumed, fuel_refunded)`

runtime then continues from saved resumable context associated with `call_id`.

## System runtime envelopes during interruption

For system-runtime addresses (`is_execute_using_system_runtime`):

- interruption output is encoded as `RuntimeInterruptionOutcomeV1`
- final execution output is encoded as `RuntimeExecutionOutcomeV1`

REVM decodes and applies these before finalizing frame result.

## Lifecycle invariants

- `call_id` is transaction-scoped and allocated by `RuntimeFactoryExecutor`.
- On final return, REVM calls `forget_runtime(call_id)`.
- `reset_call_id_counter()` is invoked per transaction and clears recoverable runtimes.
- Positive exit code means interruption id, non-positive means final halt code.
