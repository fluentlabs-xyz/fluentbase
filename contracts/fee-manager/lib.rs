#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
#![allow(clippy::assign_op_pattern)]

extern crate alloc;

use fluentbase_sdk::{
    basic_entrypoint,
    derive::{router, Contract, Event},
    storage::StorageAddress,
    Address, ContextReader, SharedAPI, DEFAULT_FEE_MANAGER_AUTH, SYSTEM_ADDRESS, U256,
};

#[derive(Event)]
struct OwnerChanged {
    new_owner: Address,
}

#[derive(Event)]
struct FeeWithdrawn {
    recipient: Address,
    amount: U256,
}

#[derive(Contract)]
struct App<SDK> {
    sdk: SDK,
    owner: StorageAddress,
}

pub trait FeeManagerTr {
    /// Withdraw balance from the contract
    fn withdraw(&mut self, recipient: Address);

    /// Change contract owner
    fn change_owner(&mut self, new_owner: Address);

    /// Get the current contract owner
    fn owner(&mut self) -> Address;

    /// Renounce ownership (change an owner to system contract address)
    fn renounce_ownership(&mut self);
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> FeeManagerTr for App<SDK> {
    fn withdraw(&mut self, recipient: Address) {
        _ = self.only_owner();
        let balance = self.only_positive_balance();
        let Ok(_) = self.sdk.call(recipient, balance, &[], None).ok() else {
            panic!("fee-manager: can't send funds to recipient");
        };
        FeeWithdrawn {
            recipient,
            amount: balance,
        }
        .emit(&mut self.sdk)
        .unwrap();
    }

    fn change_owner(&mut self, new_owner: Address) {
        _ = self.only_owner();
        self.owner_accessor().set(&mut self.sdk, new_owner);
        OwnerChanged { new_owner }.emit(&mut self.sdk).unwrap();
    }

    fn owner(&mut self) -> Address {
        let mut owner = self.owner_accessor().get(&self.sdk);
        if owner.is_zero() {
            owner = DEFAULT_FEE_MANAGER_AUTH;
        }
        owner
    }

    fn renounce_ownership(&mut self) {
        _ = self.only_owner();
        self.owner_accessor().set(&mut self.sdk, SYSTEM_ADDRESS);
        OwnerChanged {
            new_owner: SYSTEM_ADDRESS,
        }
        .emit(&mut self.sdk)
        .unwrap();
    }
}

impl<SDK: SharedAPI> App<SDK> {
    /// Only owner modifier
    fn only_owner(&self) -> Address {
        let mut owner = self.owner_accessor().get(&self.sdk);
        if owner.is_zero() {
            owner = DEFAULT_FEE_MANAGER_AUTH;
        }
        let caller = self.sdk.context().contract_caller();
        if caller != owner {
            panic!("fee-manager: incorrect caller");
        }
        owner
    }

    /// Only a positive balance modifier
    fn only_positive_balance(&self) -> U256 {
        let Ok(balance) = self.sdk.self_balance().ok() else {
            panic!("fee-manager: can't obtain self balance");
        };
        if balance.is_zero() {
            panic!("fee-manager: nothing to withdraw");
        }
        balance
    }

    pub fn deploy(&self) {
        // for system contracts deploy is not called
    }
}

basic_entrypoint!(App);
