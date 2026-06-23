#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
#![allow(unused_imports, dead_code)]

extern crate alloc;

#[cfg(test)]
mod tests;

use alloc::{borrow::Cow, string::String, vec, vec::Vec};
use fluentbase_sdk::{
    basic_entrypoint, compile_rwasm_maybe_system,
    crypto::crypto_keccak256,
    derive::{router, Contract, Event},
    hex,
    storage::{StorageAddress, StorageBytes32, StorageString, StorageVec},
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
struct ContractRecompiled {
    #[indexed]
    target_address: Address,
    code_hash: B256,
}

#[derive(Event)]
struct UpgradePlanned {
    #[indexed]
    genesis_hash: B256,
    genesis_version: String,
    target_addresses: Vec<Address>,
    wasm_code_hashes: Vec<B256>,
    updater: Address,
}

#[derive(Event)]
struct OwnerChanged {
    new_owner: Address,
}

#[derive(Contract)]
struct App<SDK> {
    sdk: SDK,
    owner: StorageAddress,
    planned_genesis_hash: StorageBytes32,
    planned_genesis_version: StorageString,
    planned_updater: StorageAddress,
    planned_target_addresses: StorageVec<StorageAddress>,
    planned_wasm_hashes: StorageVec<StorageBytes32>,
}

trait RuntimeUpgradeTr {
    /// Upgrade WASM runtime smart contract
    fn upgrade_to(
        &mut self,
        target_address: Address,
        genesis_hash: B256,
        genesis_version: String,
        wasm_bytecode: Bytes,
    );

    /// Recompile already deployed WASM runtime smart contract
    fn recompile(&mut self, target_address: Address);

    /// Plan a bulk runtime upgrade as exact target/hash pairs.
    ///
    /// The target address is part of the authorization boundary: approving only a WASM hash would
    /// let the delegated upgrader install approved bytecode at the wrong system address.
    fn plan_upgrade(
        &mut self,
        genesis_hash: B256,
        genesis_version: String,
        target_addresses: Vec<Address>,
        wasm_code_hashes: Vec<B256>,
        updater: Address,
    );

    /// Upgrade WASM runtime smart contract using a previously planned target/hash pair.
    fn upgrade_to_planned(&mut self, target_address: Address, wasm_bytecode: Bytes);

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
    fn upgrade_to(
        &mut self,
        target_address: Address,
        genesis_hash: B256,
        genesis_version: String,
        wasm_bytecode: Bytes,
    ) {
        _ = self.only_owner();
        let code_hash = self.compile_and_install(target_address, wasm_bytecode);
        RuntimeUpgraded {
            target_address,
            genesis_hash,
            genesis_version,
            code_hash,
        }
        .emit(&mut self.sdk)
        .unwrap();
    }

    #[function_id("recompile(address)")]
    fn recompile(&mut self, target_address: Address) {
        _ = self.only_owner();

        let Ok(code_size) = self.sdk.code_size(&target_address).ok() else {
            panic!("runtime-upgrade: can't obtain code size");
        };
        if code_size == 0 {
            panic!("runtime-upgrade: empty target bytecode");
        }

        let Ok(wasm_bytecode) = self
            .sdk
            .code_copy(&target_address, 0, code_size as u64)
            .ok()
        else {
            panic!("runtime-upgrade: can't load target bytecode");
        };
        if wasm_bytecode.len() != code_size as usize {
            panic!("runtime-upgrade: incomplete target bytecode");
        }

        let code_hash = self.compile_and_install(target_address, wasm_bytecode);
        ContractRecompiled {
            target_address,
            code_hash,
        }
        .emit(&mut self.sdk)
        .unwrap();
    }

    #[function_id("planUpgrade(uint256,string,address[],bytes32[],address)")]
    fn plan_upgrade(
        &mut self,
        genesis_hash: B256,
        genesis_version: String,
        target_addresses: Vec<Address>,
        wasm_code_hashes: Vec<B256>,
        updater: Address,
    ) {
        _ = self.only_owner();
        if wasm_code_hashes.is_empty() {
            panic!("runtime-upgrade: empty upgrade plan");
        }
        if target_addresses.len() != wasm_code_hashes.len() {
            panic!("runtime-upgrade: mismatched upgrade plan");
        }
        if updater == Address::ZERO {
            panic!("runtime-upgrade: planned updater is zero address");
        }

        // Validate the whole replacement plan before clearing the previous one. The runtime should
        // never persist as a partial plan if one entry is malformed.
        for (index, (target_address, wasm_code_hash)) in target_addresses
            .iter()
            .copied()
            .zip(wasm_code_hashes.iter().copied())
            .enumerate()
        {
            if target_address == Address::ZERO {
                panic!("runtime-upgrade: planned target is zero address");
            }
            if wasm_code_hash == B256::ZERO {
                panic!("runtime-upgrade: planned hash is zero");
            }
            if target_addresses[..index].contains(&target_address) {
                panic!("runtime-upgrade: duplicate planned target");
            }
        }

        self.clear_planned_hashes();
        self.planned_genesis_hash_accessor()
            .set(&mut self.sdk, genesis_hash);
        self.planned_genesis_version_accessor()
            .set(&mut self.sdk, &genesis_version);
        self.planned_updater_accessor().set(&mut self.sdk, updater);

        for (target_address, wasm_code_hash) in target_addresses
            .iter()
            .copied()
            .zip(wasm_code_hashes.iter().copied())
        {
            self.planned_target_addresses_accessor()
                .push(&mut self.sdk, target_address);
            self.planned_wasm_hashes_accessor()
                .push(&mut self.sdk, wasm_code_hash);
        }

        UpgradePlanned {
            genesis_hash,
            genesis_version,
            target_addresses,
            wasm_code_hashes,
            updater,
        }
        .emit(&mut self.sdk)
        .unwrap();
    }

    #[function_id("upgradeToPlanned(address,bytes)")]
    fn upgrade_to_planned(&mut self, target_address: Address, wasm_bytecode: Bytes) {
        let updater = self.planned_updater_accessor().get(&self.sdk);
        if updater == Address::ZERO {
            panic!("runtime-upgrade: no planned upgrade");
        }
        let caller = self.sdk.context().contract_caller();
        if caller != updater {
            panic!("runtime-upgrade: incorrect planned updater");
        }

        let wasm_code_hash = crypto_keccak256(wasm_bytecode.as_ref());
        if !self.has_planned_upgrade(target_address, wasm_code_hash) {
            panic!("runtime-upgrade: unplanned wasm hash");
        }

        let genesis_hash = self.planned_genesis_hash_accessor().get(&self.sdk);
        let genesis_version = self.planned_genesis_version_accessor().get(&self.sdk);
        let code_hash = self.compile_and_install(target_address, wasm_bytecode);
        // Consume the exact target/hash pair so a planned upgrade cannot be replayed.
        self.remove_planned_upgrade(target_address, wasm_code_hash);

        RuntimeUpgraded {
            target_address,
            genesis_hash,
            genesis_version,
            code_hash,
        }
        .emit(&mut self.sdk)
        .unwrap();
    }

    #[function_id("changeOwner(address)")]
    fn change_owner(&mut self, new_owner: Address) {
        _ = self.only_owner();
        if new_owner == Address::ZERO {
            panic!("runtime-upgrade: can't set owner to zero address");
        }
        self.owner_accessor().set(&mut self.sdk, new_owner);
        OwnerChanged { new_owner }.emit(&mut self.sdk).unwrap();
    }

    #[function_id("owner()")]
    fn owner(&mut self) -> Address {
        let mut owner = self.owner_accessor().get(&self.sdk);
        if owner.is_zero() {
            owner = DEFAULT_UPDATE_GENESIS_AUTH;
        }
        owner
    }

    #[function_id("renounceOwnership()")]
    fn renounce_ownership(&mut self) {
        _ = self.only_owner();
        // We set to `SYSTEM_ADDRESS` to make a system fully maintained by forks (if it's required)
        self.owner_accessor().set(&mut self.sdk, SYSTEM_ADDRESS);
        OwnerChanged {
            new_owner: SYSTEM_ADDRESS,
        }
        .emit(&mut self.sdk)
        .unwrap();
    }
}

impl<SDK: SharedAPI> App<SDK> {
    fn compile_and_install(&mut self, target_address: Address, wasm_bytecode: Bytes) -> B256 {
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

        code_hash
    }

    fn clear_planned_hashes(&mut self) {
        self.planned_target_addresses_accessor()
            .clear(&mut self.sdk);
        self.planned_wasm_hashes_accessor().clear(&mut self.sdk);
    }

    fn has_planned_upgrade(&self, target_address: Address, wasm_code_hash: B256) -> bool {
        let planned_target_addresses = self.planned_target_addresses_accessor();
        let planned_wasm_hashes = self.planned_wasm_hashes_accessor();
        let hashes_len = planned_wasm_hashes.len(&self.sdk);
        for index in 0..hashes_len {
            if planned_target_addresses.at(index).get(&self.sdk) == target_address
                && planned_wasm_hashes.at(index).get(&self.sdk) == wasm_code_hash
            {
                return true;
            }
        }
        false
    }

    fn remove_planned_upgrade(&mut self, target_address: Address, wasm_code_hash: B256) {
        let planned_target_addresses = self.planned_target_addresses_accessor();
        let planned_wasm_hashes = self.planned_wasm_hashes_accessor();
        let hashes_len = planned_wasm_hashes.len(&self.sdk);
        for index in 0..hashes_len {
            if planned_target_addresses.at(index).get(&self.sdk) != target_address
                || planned_wasm_hashes.at(index).get(&self.sdk) != wasm_code_hash
            {
                continue;
            }

            let last_index = hashes_len - 1;
            if index != last_index {
                let last_target = planned_target_addresses.at(last_index).get(&self.sdk);
                let last_hash = planned_wasm_hashes.at(last_index).get(&self.sdk);
                planned_target_addresses
                    .at(index)
                    .set(&mut self.sdk, last_target);
                planned_wasm_hashes.at(index).set(&mut self.sdk, last_hash);
            }
            _ = planned_target_addresses.pop(&mut self.sdk);
            _ = planned_wasm_hashes.pop(&mut self.sdk);
            break;
        }
    }

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
