# fluentbase-runtime

> NOTE: syscall extensions: bla12381, bn254, secp256k1, secp2561 will be removed as it's
> duplicated by weierstrass

A minimal execution environment for running rWASM smart contracts in Fluentbase. It wires the rWASM engine with
Fluentbase host syscalls, provides deterministic fuel (gas) accounting, supports resumable execution, and caches
compiled modules.

What this crate is (in short):

- Host surface for contracts compiled to rWASM (syscall dispatch, crypto/hashing, IO).
- A small executor API to run or resume contracts with precise fuel control.
- A module factory with per-code-hash caches (and optional Wasmtime artifacts) to reduce cold-start costs.

Core concepts

- RuntimeContext: Per-invocation state (fuel_limit, state selector, call_depth, input, output/return buffers). It can
  run with fuel metering disabled for proof or special flows.
- RuntimeExecutor: Trait providing execute and resume. default_runtime_executor() returns the default executor used by
  the project.
- ExecutionMode: Internal enum for concrete engines. Strategy and Rwasm are available; Wasmtime is available behind the
  wasmtime feature.
- Syscall handler: Central dispatcher mapping SysFuncIdx to handlers for IO, hashing, curves, bigint, and control (exit,
  exec, resume, fuel).
- ModuleFactory: Global, lazy-initialized cache keyed by code hash. Stores rWASM modules and, when enabled, compiled
  Wasmtime modules. Provides warmup hooks.

Execution flow

1) Prepare a RuntimeContext with the desired fuel limit, state (entry selector), and input bytes.
2) Call RuntimeExecutor::execute with BytecodeOrHash (either the module and its code hash, or a code hash when already
   cached).
3) You receive an ExecutionResult (completed) or an interruption encoded as a positive exit_code that acts as a call_id
   to resume later.
4) To continue after an interruption (e.g., delegated call), call RuntimeExecutor::resume with the call_id, return_data,
   and fuel bookkeeping.

Fuel model

- Fuel corresponds to deterministic metering used by rWASM. RuntimeContext.fuel_limit bounds execution. disable_fuel
  lets builtins manage fuel manually.
- Syscalls include CHARGE_FUEL and CHARGE_FUEL_MANUALLY, plus FUEL to query remaining fuel.
- ExecutionResult exposes fuel_consumed and fuel_refunded (in fuel units). Gas conversion is the caller’s
  responsibility.

Host interface (syscalls)
Grouped by SysFuncIdx categories exposed to rWASM modules:

- Control/IO: EXIT, STATE, READ_INPUT, INPUT_SIZE, WRITE_OUTPUT, OUTPUT_SIZE, READ_OUTPUT, EXEC, RESUME, FORWARD_OUTPUT,
  DEBUG_LOG
- Fuel: CHARGE_FUEL, CHARGE_FUEL_MANUALLY, FUEL
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

Module caching and warmup

- ModuleFactory::get_module_or_init compiles/caches rWASM modules by code hash.
- With the wasmtime feature, get_wasmtime_module_or_compile and warmup_wasmtime allow precompilation and warming the
  caches to eliminate first-run latency.

Feature flags

- std (default): Enables std dependencies in this crate and transitively in rwasm and fluentbase-types.
- wasmtime: Enables an alternative engine for debugging/profiling, plus compiled-artifact caching.
- inter-process-lock: Enables an fs2-based lock used by certain wasmtime paths.
- global-executor: Enables optional global executor helpers (not fully tested).
- debug-print, rwasm: Internal toggles.

Notes

- This crate intentionally does not define storage/account models; it focuses on execution, fuel, and host syscalls.
  Integrators plug their own state layers above this runtime.
- For debugging and developer ergonomics, enable the wasmtime feature; for proofs/no_std environments, disable default
  features and build accordingly.

Part of the Fluentbase project: https://github.com/fluentlabs-xyz/fluentbase
