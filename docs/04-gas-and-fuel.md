# Gas and Fuel Accounting

## Units

- **EVM gas**: REVM-visible execution accounting.
- **Fuel**: runtime/rWasm execution accounting.

Conversion constant (`crates/types/src/lib.rs`):

- `FUEL_DENOM_RATE = 20`
- conversion used throughout:
  - `fuel_limit = gas_remaining * FUEL_DENOM_RATE`
  - `gas_consumed = ceil(fuel_consumed / FUEL_DENOM_RATE)`
  - `gas_refund += fuel_refunded / FUEL_DENOM_RATE`

## Where conversion is applied

In `crates/revm/src/executor.rs`:

- before runtime call: gas -> fuel limit
- after `_exec`: fuel consumed/refunded -> REVM gas charge/refund
- after `_resume`: same conversion and settlement path

## EVM runtime internal syncing

`contracts/evm` uses interruptible EVM and tracks `committed_gas` vs current gas.

- `ExecutionResult::chargeable_fuel()` (`crates/evm/src/types.rs`) computes delta fuel from remaining-gas difference.
- `EthVM::sync_evm_gas(...)` (`crates/evm/src/evm.rs`) charges host fuel for committed delta before interruption/finalization.

## Import syscall fuel schedules

Runtime import-level fuel procedures are preconfigured in `crates/types/src/block_fuel.rs`:

- const/linear/quadratic fuel policies per `SysFuncIdx`
- e.g. `_exec` uses quadratic policy
- copy/hash/log syscalls use linear policy with per-word cost

This fuel is inserted in translated rWasm execution path.

## System runtime metering modes

From `compile_rwasm_maybe_system` (`crates/sdk/src/types/rwasm.rs`) and `SystemRuntime`:

- some system runtimes are **self-metered** (`consume_fuel=false`)
- selected runtimes are **engine-metered** (`consume_fuel=true`) via `is_engine_metered_precompile`

`is_engine_metered_precompile` currently includes:
- Nitro verifier
- OAuth2 verifier
- Wasm runtime
- WebAuthn verifier
- Universal token runtime

## Call data surcharge

`crates/types/src/block_fuel.rs` defines quadratic surcharge above 128 KiB:

- threshold: `CALLDATA_QUADRATIC_THRESHOLD = 128 * 1024`
- surcharge: `3*words + words^2 / CALLDATA_QUADRATIC_DIVISOR`
- divisor: `30`

## Practical invariants

- Never allocate host buffers from untrusted lengths before bounds/gas checks.
- Charge REVM gas before expensive host operations where possible.
- Keep conversion and rounding behavior deterministic (especially `div_ceil`).
