use fluentbase_sdk::{derive::solidity_storage, Address, SharedAPI, U256};

solidity_storage! {
    mapping(Address => U256) Balance;
    mapping(Address => mapping(Address => U256)) Allowance;
}

impl Balance {
    pub fn add(sdk: &mut impl SharedAPI, address: Address, amount: U256) {
        let current_balance = Self::get(sdk, address);
        let new_balance = current_balance + amount;
        Self::set(sdk, address, new_balance);
    }

    pub fn subtract(sdk: &mut impl SharedAPI, address: Address, amount: U256) -> bool {
        let current_balance = Self::get(sdk, address);
        if current_balance < amount {
            return false;
        }
        let new_balance = current_balance - amount;
        Self::set(sdk, address, new_balance);
        true
    }
}

impl Allowance {
    #[allow(unused)]
    pub fn add(sdk: &mut impl SharedAPI, owner: Address, spender: Address, amount: U256) {
        let current_allowance = Self::get(sdk, owner, spender);
        let new_allowance = current_allowance + amount;
        Self::set(sdk, owner, spender, new_allowance);
    }

    pub fn subtract(
        sdk: &mut impl SharedAPI,
        owner: Address,
        spender: Address,
        amount: U256,
    ) -> bool {
        let current_allowance = Self::get(sdk, owner, spender);
        if current_allowance < amount {
            return false;
        }
        let new_allowance = current_allowance - amount;
        Self::set(sdk, owner, spender, new_allowance);
        true
    }
}
