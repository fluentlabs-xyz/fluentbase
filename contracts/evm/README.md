# EVM

A minimal contract wrapper that embeds the fluentbase-evm interpreter into a smart-contract friendly entrypoint. It
handles two flows:

- `deploy_entry`: runs EVM init code and commits the resulting runtime bytecode to metadata.
- `main_entry`: executes previously deployed bytecode with the provided call data.

This crate does not implement EVM itself — it wires the host (SharedAPI) to the interpreter from crates/evm and applies
basic network rules relevant to deployment.

## Architecture

- Interpreter: provided by fluentbase-evm (an interruptible, bytecode-compatible EVM engine).
- Host: provided by SharedAPI (I/O, fuel charging, metadata access). Calls, storage, logs, etc. are executed via
  interruptions and resumed transparently by the VM.
- Metadata: contract-local storage used to persist code hash and raw bytecode.

## Entrypoints

- `deploy_entry`: executes init bytecode, enforces EIP-3541 (no 0xEF prefix) and EIP-170 (code size), charges
  CODEDEPOSIT,
  then stores code hash (offset 0) and raw bytecode (offset 32) in metadata.
- `main_entry`: loads analyzed bytecode from metadata, runs the interpreter with call data, settles fuel delta, and
  writes
  return data.

Relevant functions in lib.rs:

- commit_evm_bytecode: persist code hash and raw bytecode in metadata.
- load_evm_bytecode: read code hash + bytes from metadata and return AnalyzedBytecode.
- handle_not_ok_result: charge final fuel delta, write output, and exit with Err/Panic for non-success.

## Bytecode lifecycle

1) Deployment (deploy_entry)
    - Input: init bytecode (Bytes) from sdk.input().
    - Run EthVM once; the output is the runtime bytecode.
    - Validate: reject 0xEF prefix (EIP-3541) and limit size to EVM_MAX_CODE_SIZE (EIP-170).
    - Charge CODEDEPOSIT = len(runtime) * gas::CODEDEPOSIT.
    - Commit: write keccak256(runtime) to metadata at offset 0; write runtime bytes at offset 32.

2) Execution (main_entry)
    - Load AnalyzedBytecode from metadata (code hash + byte array).
    - Run EthVM with call data (sdk.bytes_input()).
    - Settle fuel delta via ExecutionResult::chargeable_fuel_and_refund and write output.

## Gas/fuel model

- The interpreter tracks EVM gas; the host may charge “fuel”. fluentbase-evm keeps both in sync using FUEL_DENOM_RATE.
- This crate only settles the final delta at the end of each entrypoint via ExecutionResult::chargeable_fuel_and_refund.

## Host expectations (SharedAPI)

- metadata_write, metadata_copy, metadata_size for persisting bytecode and its hash.
- context() for addresses and limits; bytes_input()/input() for calldata and initcode.
- charge_fuel_manually and native_exit for accounting and termination.

## Usage

- Include this crate in contracts/* workspace.
- Call deploy_entry on creation, main_entry on calls. The entrypoint! macro at the bottom of lib.rs wires both.

## Notes

- The EVM specification and gas semantics are provided by fluentbase-evm (Prague by default). This crate only manages
  the bytecode lifecycle and host boundary.