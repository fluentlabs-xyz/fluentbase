# fluentbase-runtime

> NOTE: syscall extensions `bls12381`, `bn254`, `secp256k1`, and `secp256r1` are planned for
> consolidation with the weierstrass implementation.

A minimal execution environment for running rWASM smart contracts in Fluentbase. It wires the rWASM engine with
Fluentbase host syscalls, provides deterministic fuel (gas) accounting, supports resumable execution, and caches
compiled modules.

What this crate is (in short):

- Host surface for contracts compiled to rWASM (syscall dispatch, crypto/hashing, IO).
- A small executor API to run or resume contracts with precise fuel control.
- A per-thread cache of compiled system runtimes to reduce their cold-start cost.

Core concepts

- RuntimeContext: Per-invocation state (fuel_limit, state selector, call_depth, input, output/return buffers). It can
  run with fuel metering disabled for proof or special flows.
- RuntimeExecutor: Trait providing execute and resume. default_runtime_executor() returns the default executor used by
  the project.
- ExecutionMode: Internal enum for concrete engines. Strategy and Rwasm are available; Wasmtime is available behind the
  wasmtime feature.
- Syscall handler: Central dispatcher mapping SysFuncIdx to handlers for IO, hashing, curves, bigint, and control (exit,
  exec, resume, fuel).
- SystemRuntime: Trusted runtimes (EVM, SVM, verifiers) run through a per-thread cache of compiled instances keyed by
  code hash and compilation-config fingerprint.

Execution flow

1) Prepare a RuntimeContext with the desired fuel limit, state (entry selector), and input bytes.
2) Call RuntimeExecutor::execute with BytecodeOrHash::Bytecode carrying the parsed module, its code hash, and the
   account address. The runtime keeps no module cache, so a bare BytecodeOrHash::Hash is rejected with
   UnexpectedFatalExecutionFailure.
3) You receive an ExecutionResult (completed) or an interruption encoded as a positive exit_code that acts as a call_id
   to resume later.
4) To continue after an interruption (e.g., delegated call), call RuntimeExecutor::resume with the call_id, return_data,
   and fuel bookkeeping.

Fuel model

- Fuel corresponds to deterministic metering used by rWASM. RuntimeContext.fuel_limit bounds execution. disable_fuel
  lets builtins manage fuel manually.
- Syscalls include CHARGE_FUEL, plus FUEL to query remaining fuel.
- ExecutionResult exposes fuel_consumed and fuel_refunded (in fuel units). Gas conversion is the caller’s
  responsibility.

Host interface (syscalls)
Grouped by SysFuncIdx categories exposed to rWASM modules:

- Control/IO: EXIT, STATE, READ_INPUT, INPUT_SIZE, WRITE_OUTPUT, OUTPUT_SIZE, READ_OUTPUT, EXEC, RESUME, FORWARD_OUTPUT,
  DEBUG_LOG
- Fuel: CHARGE_FUEL, FUEL
- Preimage: PREIMAGE_SIZE, PREIMAGE_COPY
- Hashing: KECCAK256, KECCAK256_PERMUTE, SHA256, SHA256_EXTEND, SHA256_COMPRESS, BLAKE3, POSEIDON
- Curves/crypto: ed25519 (add, sub, mul, msm, decompress), ristretto255, secp256k1 (recover, add, double, decompress),
  secp256r1 (verify)
- Pairing-friendly: bls12-381 (G1/G2 ops, MSM, pairing, map), bn254 (G1/G2 ops, fp/fp2 ops, pairing)
- Big integer: BIGINT_MOD_EXP, BIGINT_UINT256_MUL

Resumable execution

- When a contract yields (e.g., via EXEC/RESUME patterns), the runtime returns an interruption. The
  RuntimeFactoryExecutor stores the suspended engine under a per-transaction call_id.
- The positive exit_code returned to the caller is that call_id. Use it with RuntimeExecutor::resume to continue
  execution, supplying any return_data and fuel adjustments.
- RuntimeExecutor::reset_call_id_counter clears suspended runtimes at the start of a new transaction.

Module ownership and caching

- The caller owns rWASM modules. REVM passes the parsed module of the account it executes on every call, so the runtime
  never resolves a module by code hash alone and keeps no process-wide module cache.
- System runtimes are compiled once per thread and reused across calls (SystemRuntime::COMPILED_RUNTIMES), keyed by
  code hash and compilation-config fingerprint. An unexpected trap evicts the cached instance.

Feature flags

- std (default): Enables std dependencies in this crate and transitively in rwasm and fluentbase-types.
- wasmtime: Backs system runtimes with the Wasmtime engine instead of the rWASM interpreter.
- debug-print, rwasm: Internal toggles.

Notes

- This crate intentionally does not define storage/account models; it focuses on execution, fuel, and host syscalls.
  Integrators plug their own state layers above this runtime.
- For debugging and developer ergonomics, enable the wasmtime feature; for proofs/no_std environments, disable default
  features and build accordingly.

Part of the Fluentbase project: https://github.com/fluentlabs-xyz/fluentbase
