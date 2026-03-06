#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
#![allow(unused_imports, dead_code)]

extern crate alloc;

use alloc::{borrow::Cow, string::String, vec};
use fluentbase_sdk::{
    basic_entrypoint,
    codec::Codec,
    compile_rwasm_maybe_system, debug_log,
    derive::{function_id, router, Contract, Event},
    hex,
    storage::StorageAddress,
    syscall::{encode, SYSCALL_ID_UPGRADE_RUNTIME},
    Address, Bytes, ContextReader, ExitCode, RwasmCompilationResult, SharedAPI, B256,
    DEFAULT_UPDATE_GENESIS_AUTH, STATE_MAIN, SYSTEM_ADDRESS, WASM_MAGIC_BYTES,
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

#[derive(Event)]
struct OwnerChanged {
    new_owner: Address,
}

#[derive(Contract)]
struct App<SDK> {
    sdk: SDK,
    owner: StorageAddress,
}

#[derive(Debug, Default, Codec, PartialEq, Clone)]
pub struct UpgradeToInput {
    pub target_address: Address,
    pub genesis_hash: B256,
    pub genesis_version: String,
    pub wasm_bytecode: Bytes,
}

trait RuntimeUpgradeTr {
    /// Upgrade WASM runtime smart contract
    fn upgrade_to(&mut self, input: UpgradeToInput);

    /// Change contract owner
    fn change_owner(&mut self, new_owner: Address);

    /// Get the current contract owner
    fn owner(&mut self) -> Address;

    /// Renounce ownership (change an owner to system contract address)
    fn renounce_ownership(&mut self);
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> RuntimeUpgradeTr for App<SDK> {
    #[function_id("upgradeTo(address,uint256,string,bytes)")]
    fn upgrade_to(&mut self, input: UpgradeToInput) {
        let UpgradeToInput {
            target_address,
            genesis_hash,
            genesis_version,
            wasm_bytecode,
        } = input;
        _ = self.only_owner();
        debug_log!("WASM bytecode: {}", hex::encode(&wasm_bytecode));
        if !wasm_bytecode.starts_with(&WASM_MAGIC_BYTES) {
            panic!("runtime-upgrade: malformed wasm bytecode");
        }
        let Ok(RwasmCompilationResult { rwasm_module, .. }) =
            compile_rwasm_maybe_system(&target_address, &wasm_bytecode)
        else {
            panic!("runtime-upgrade: failed to compile bytecode");
        };
        let rwasm_bytecode = rwasm_module.serialize();

        let mut buffer = vec![0u8; encode::upgrade_runtime_size_hint(rwasm_bytecode.len())];
        encode::upgrade_runtime_into(&mut &mut buffer[..], &target_address, &rwasm_bytecode);
        let (_fuel_consumed, _fuel_refunded, exit_code) = self.sdk.native_exec(
            SYSCALL_ID_UPGRADE_RUNTIME,
            Cow::Owned(buffer),
            None,
            STATE_MAIN,
        );

        if exit_code != ExitCode::Ok.into_i32() {
            panic!("runtime-upgrade: failed to upgrade");
        }

        let Ok(code_hash) = self.sdk.code_hash(&target_address).ok() else {
            panic!("runtime-upgrade: can't obtain code hash");
        };

        RuntimeUpgraded {
            target_address,
            genesis_hash,
            genesis_version,
            code_hash,
        }
        .emit(&mut self.sdk);
    }

    fn change_owner(&mut self, new_owner: Address) {
        _ = self.only_owner();
        self.owner_accessor().set(&mut self.sdk, new_owner);
        OwnerChanged { new_owner }.emit(&mut self.sdk);
    }

    fn owner(&mut self) -> Address {
        let mut owner = self.owner_accessor().get(&self.sdk);
        if owner.is_zero() {
            owner = DEFAULT_UPDATE_GENESIS_AUTH;
        }
        owner
    }

    fn renounce_ownership(&mut self) {
        _ = self.only_owner();
        self.owner_accessor().set(&mut self.sdk, SYSTEM_ADDRESS);
        OwnerChanged {
            new_owner: SYSTEM_ADDRESS,
        }
        .emit(&mut self.sdk);
    }
}

impl<SDK: SharedAPI> App<SDK> {
    fn only_owner(&self) -> Address {
        let mut owner = self.owner_accessor().get(&self.sdk);
        if owner == Address::ZERO {
            owner = DEFAULT_UPDATE_GENESIS_AUTH;
        }
        let caller = self.sdk.context().contract_caller();
        if caller != owner {
            panic!("runtime-upgrade: incorrect caller");
        }
        owner
    }

    pub fn deploy(&self) {
        // system contracts don't have a `deploy` stage (keep empty)
    }
}

basic_entrypoint!(App);
