Compatibility Layer
===================

Compatibility layer is a special verified WASM binary that is responsible for executing runtime ahead of LowLevelAPI.
The most common example of CL is EVM that can be used to simulate the most important EVM methods behaviour.

Core Layer
==========

Methods:
- `_sys_halt`
- `_sys_input_size`
- `_sys_read`
- `_sys_write`
- `_sys_exec2(bytecode_hash, input)`
- `_sys_exec(bytecode, input)`

```Rust
// WASM -> rWASM
// EVM  -> rWASM

fn _statedb_change_bytecode(...) {
    values[3] = poseidon(bytecode);
    _zktrie_update_leaf(key, flags, values)
}

fn _evm_create(...) {
    let address = keccak256(address + nonce);
    let result = _sys_exec(bytecode);
    _statedb_change_bytecode(address, result);
}

static SOME_DATA: Vec<u8> = Vec::new();

fn _evm_return(data: &[u8]) {
    // save somewhere data
    SOME_DATA = data;
}

fn _return_data_size() -> usize {
    SOME_DATA.len()
}

struct ModuleCallInput {
    method_id: u32, // 75 (_evm_create)
    input_data: Bytes,
}
```

EVM Compatibility Layer
=======================

Methods:
- `_evm_create/_evm_create2`