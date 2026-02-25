#![cfg_attr(target_arch = "wasm32", no_std, no_main)]

extern crate alloc;

use alloc::{borrow::Cow, string::String, vec};
use fluentbase_sdk::{
    codec::SolidityABI,
    derive::Event,
    entrypoint,
    syscall::{encode, SYSCALL_ID_UPGRADE_RUNTIME},
    Address, Bytes, ContextReader, ExitCode, SharedAPI, B256, STATE_MAIN, UPDATE_GENESIS_AUTH,
    UPDATE_GENESIS_PREFIX, WASM_MAGIC_BYTES,
};

#[derive(Event)]
struct RuntimeUpgraded {
    #[indexed]
    target_address: Address,
    #[indexed]
    genesis_hash: B256,
    genesis_version: String,
    code_hash: B256,
}

pub fn main_entry<SDK: SharedAPI>(mut sdk: SDK) {
    let caller = sdk.context().contract_caller();
    if caller != UPDATE_GENESIS_AUTH {
        panic!("runtime-upgrade: incorrect caller");
    }

    let input = sdk.bytes_input();
    if !input.starts_with(&UPDATE_GENESIS_PREFIX) {
        panic!("runtime-upgrade: unknown method");
    }

    let (target_address, genesis_hash, genesis_version, wasm_bytecode) =
        SolidityABI::<(Address, B256, String, Bytes)>::decode(&input.slice(4..), 0).unwrap();

    if !wasm_bytecode.starts_with(&WASM_MAGIC_BYTES) {
        panic!("runtime-upgrade: malformed wasm bytecode");
    }

    let mut buffer = vec![0u8; encode::upgrade_runtime_size_hint(wasm_bytecode.len())];
    encode::upgrade_runtime_into(&mut &mut buffer[..], &target_address, &wasm_bytecode);
    let (_fuel_consumed, _fuel_refunded, exit_code) = sdk.native_exec(
        SYSCALL_ID_UPGRADE_RUNTIME,
        Cow::Owned(buffer),
        None,
        STATE_MAIN,
    );

    if exit_code != ExitCode::Ok.into_i32() {
        panic!("runtime-upgrade: failed to upgrade");
    }

    let Ok(code_hash) = sdk.code_hash(&target_address).ok() else {
        panic!("runtime-upgrade: can't obtain code hash");
    };

    RuntimeUpgraded {
        target_address,
        genesis_hash,
        genesis_version,
        code_hash,
    }
    .emit(&mut sdk);
}

entrypoint!(main_entry);
