# Syscall Surface (Core)

Fluentbase has two syscall layers:

1. **Runtime import syscalls** (`fluentbase_v1preview`), indexed by `SysFuncIdx`.
2. **REVM interruption syscalls** (`SYSCALL_ID_*`, `B256`) used between runtime and host handler.

---

## 1) Runtime import syscalls (`SysFuncIdx`)

Defined in:
- `crates/types/src/sys_func_idx.rs`
- `crates/types/src/import_linker.rs`

Important control/data syscalls:

- `_read`, `_input_size`
- `_write`, `_output_size`, `_read_output`, `_forward_output`
- `_exec`, `_resume`
- `_fuel`, `_charge_fuel`
- `_state`
- `_debug_log`

Notes:
- `_write_fd` is currently disabled in linker (`import_linker_v1_preview`).
- Fuel procedures for each imported syscall are configured by `calculate_syscall_fuel(...)` in `crates/types/src/block_fuel.rs`.

---

## 2) REVM interruption syscalls (`SYSCALL_ID_*`)

Defined in `crates/sdk/src/syscall.rs`.
Handled in `crates/revm/src/syscall.rs`.

### Storage and state
- `SYSCALL_ID_STORAGE_READ`
- `SYSCALL_ID_STORAGE_WRITE`
- `SYSCALL_ID_TRANSIENT_READ`
- `SYSCALL_ID_TRANSIENT_WRITE`
- `SYSCALL_ID_BLOCK_HASH`

### Calls / creates / lifecycle
- `SYSCALL_ID_CALL`
- `SYSCALL_ID_STATIC_CALL`
- `SYSCALL_ID_CALL_CODE`
- `SYSCALL_ID_DELEGATE_CALL`
- `SYSCALL_ID_CREATE`
- `SYSCALL_ID_CREATE2`
- `SYSCALL_ID_DESTROY_ACCOUNT`

### Code/account queries
- `SYSCALL_ID_BALANCE`
- `SYSCALL_ID_SELF_BALANCE`
- `SYSCALL_ID_CODE_SIZE`
- `SYSCALL_ID_CODE_HASH`
- `SYSCALL_ID_CODE_COPY`

### Metadata/ownership surface
- `SYSCALL_ID_METADATA_SIZE`
- `SYSCALL_ID_METADATA_ACCOUNT_OWNER`
- `SYSCALL_ID_METADATA_CREATE`
- `SYSCALL_ID_METADATA_WRITE`
- `SYSCALL_ID_METADATA_COPY`
- `SYSCALL_ID_METADATA_STORAGE_READ`
- `SYSCALL_ID_METADATA_STORAGE_WRITE`

### Governance
- `SYSCALL_ID_UPGRADE_RUNTIME`

---

## Important rules currently enforced by handler

- Input length + state checks (`STATE_MAIN`) are validated before execution.
- Many mutating syscalls reject static context (`StateChangeDuringStaticCall`).
- `CODE_COPY` is hard-capped by `EXT_CODE_COPY_MAX_COPY_SIZE`.
- Metadata mutation requires same ownable runtime owner.
- Upgrade syscall is restricted to `PRECOMPILE_RUNTIME_UPGRADE` target.

---

## Where to extend safely

When adding syscall behavior, update together:

1. `crates/sdk/src/syscall.rs` (ID)
2. `crates/revm/src/syscall.rs` (host semantics)
3. `crates/types/src/import_linker.rs` and `block_fuel.rs` (if import-level syscall)
4. tests in `crates/revm/src/tests.rs` / `e2e/*`
