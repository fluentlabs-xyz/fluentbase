# Runtime Upgrade Flow

## Components

- Upgrade contract: `contracts/runtime-upgrade/lib.rs`
- Host syscall handler: `SYSCALL_ID_UPGRADE_RUNTIME` in `crates/revm/src/syscall.rs`
- Upgrade syscall ID: `crates/sdk/src/syscall.rs`
- Authority defaults/constants: `crates/types/src/genesis.rs`

## On-chain API (runtime-upgrade contract)

Main functions:

- `upgradeTo(address target, uint256 genesisHash, string genesisVersion, bytes wasmBytecode)`
- `changeOwner(address)`
- `owner()`
- `renounceOwnership()`

Behavior:

1. only owner can call `upgradeTo`.
2. input wasm must start with wasm magic.
3. wasm is compiled to rWasm with `compile_rwasm_maybe_system`.
4. contract calls native syscall `SYSCALL_ID_UPGRADE_RUNTIME` with:
   - target address
   - serialized rWasm bytecode
5. emits `RuntimeUpgraded` event.

## Host-side restrictions (`SYSCALL_ID_UPGRADE_RUNTIME`)

`crates/revm/src/syscall.rs` enforces:

- not allowed in static context,
- callable only when `current_target_address == PRECOMPILE_RUNTIME_UPGRADE`,
- payload must decode target address + valid rWasm bytecode,
- target account is loaded and code replaced with `Bytecode::Rwasm`.

## Legacy testnet hook

`crates/revm/src/evm.rs` still contains testnet-only special upgrade hook path gated by chain ids `0x5201/0x5202` and auth prefix checks.
This path is explicitly documented in-code as temporary.

## Ownership semantics

In runtime-upgrade contract:

- storage owner defaults to `DEFAULT_UPDATE_GENESIS_AUTH` when unset,
- `renounceOwnership()` sets owner to `SYSTEM_ADDRESS`,
- `changeOwner(0)` is rejected.

## Operational requirements

Any runtime upgrade process should include:

- deterministic build artifact hashes,
- multisig-controlled owner account,
- rollout plan for all nodes,
- post-upgrade verification (`code_hash` and behavior checks).
