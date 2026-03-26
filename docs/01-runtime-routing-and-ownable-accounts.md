# Runtime Routing and Ownable Accounts

## Ownable account bytecode format

Defined in upstream revm bytecode crate (`revm-rwasm/.../bytecode/src/ownable_account.rs`):

`0xEF44 || version:u8 || owner_address:20B || metadata:bytes`

- magic: `0xEF44`
- supported version: `0`
- `owner_address` selects delegated runtime/account owner
- `metadata` is runtime-controlled payload

## Create-time routing

Implemented in `RwasmEvm::frame_init` (`crates/revm/src/evm.rs`):

1. For every `FrameInput::Create`, the init code is inspected by
   `resolve_precompiled_runtime_from_input(...)` (`crates/types/src/genesis.rs`).
2. Resolver rules:
   - wasm magic => `PRECOMPILE_WASM_RUNTIME`
   - svm ELF magic (feature-gated) => `PRECOMPILE_SVM_RUNTIME`
   - universal token magic => `PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME`
   - fallback => `PRECOMPILE_EVM_RUNTIME`
3. Created account code is replaced with `Bytecode::OwnableAccount(owner=resolved_runtime)`.
4. Original init code is forwarded as runtime input (constructor payload).

## Runtime execution from ownable accounts

In `execute_rwasm_frame` (`crates/revm/src/executor.rs`):

- If frame bytecode is `OwnableAccount`, REVM loads code of `owner_address` and executes that code.
- `account_owner` is attached to interpreter input for downstream checks and tracing.

## Access restrictions enforced by host side

`execute_rwasm_frame` rejects direct execution where target or bytecode address is a delegated runtime address:

- `is_delegated_runtime_address(target)` => reject
- `is_delegated_runtime_address(bytecode_address)` => reject

This prevents direct user calls into delegated runtime addresses as regular contracts.

## Metadata syscalls and ownership checks

Metadata-related syscalls are handled in `crates/revm/src/syscall.rs`:

- `SYSCALL_ID_METADATA_SIZE`
- `SYSCALL_ID_METADATA_CREATE`
- `SYSCALL_ID_METADATA_WRITE`
- `SYSCALL_ID_METADATA_COPY`
- `SYSCALL_ID_METADATA_ACCOUNT_OWNER`
- `SYSCALL_ID_METADATA_STORAGE_READ/WRITE`

Security rule used by create/write/copy paths:
- operation is allowed only when target account code is `OwnableAccount`
- and `ownable.owner_address == caller account_owner_address`

Static-call protection:
- metadata mutation syscalls (`CREATE/WRITE/STORAGE_WRITE`) reject `is_static`.

## Constructor rewrite for wasm wrapper runtime

`run_rwasm_loop` has an additional deploy-time rewrite path (`crates/revm/src/executor.rs`):

- if create output begins with `0xEF` and account is delegated runtime ownable account,
  REVM may parse output as rWasm module + constructor tail,
  replace deployed code with `Bytecode::Rwasm`,
  then re-run deploy logic with remaining constructor params.

This is used by `contracts/wasm` runtime pipeline.
