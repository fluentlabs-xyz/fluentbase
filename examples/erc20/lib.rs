#![cfg_attr(target_arch = "wasm32", no_std)]
#![allow(dead_code)]
extern crate alloc;
extern crate fluentbase_sdk;

use alloc::vec::Vec;
use alloy_sol_types::SolEvent;
use fluentbase_sdk::{
    basic_entrypoint,
    derive::{router, solidity_storage},
    Address,
    Bytes,
    ContractContextReader,
    SharedAPI,
    B256,
    U256,
};

pub trait ERC20API {
    fn symbol(&self) -> Bytes;
    fn name(&self) -> Bytes;
    fn decimals(&self) -> U256;
    fn total_supply(&self) -> U256;
    fn balance_of(&self, address: Address) -> U256;
    fn transfer(&mut self, to: Address, value: U256) -> U256;
    fn allowance(&self, owner: Address, spender: Address) -> U256;
    fn approve(&mut self, spender: Address, value: U256) -> U256;
    fn transfer_from(&mut self, from: Address, to: Address, value: U256) -> U256;
}

// Define the Transfer and Approval events
sol! {
    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);
}

fn emit_event<SDK: SharedAPI, T: SolEvent>(sdk: &mut SDK, event: T) {
    let data: Bytes = event.encode_data().into();
    let topics: Vec<B256> = event
        .encode_topics()
        .iter()
        .map(|v| B256::from(v.0))
        .collect();
    sdk.emit_log(data, &topics);
}

solidity_storage! {
    mapping(Address => U256) Balance;
    mapping(Address => mapping(Address => U256)) Allowance;
}

impl Balance {
    fn add<SDK: SharedAPI>(
        sdk: &mut SDK,
        address: Address,
        amount: U256,
    ) -> Result<(), &'static str> {
        let current_balance = Self::get(sdk, address);
        let new_balance = current_balance + amount;
        Self::set(sdk, address, new_balance);
        Ok(())
    }
    fn subtract<SDK: SharedAPI>(
        sdk: &mut SDK,
        address: Address,
        amount: U256,
    ) -> Result<(), &'static str> {
        let current_balance = Self::get(sdk, address);
        if current_balance < amount {
            return Err("insufficient balance");
        }
        let new_balance = current_balance - amount;
        Self::set(sdk, address, new_balance);
        Ok(())
    }
}

impl Allowance {
    fn add<SDK: SharedAPI>(
        sdk: &mut SDK,
        owner: Address,
        spender: Address,
        amount: U256,
    ) -> Result<(), &'static str> {
        let current_allowance = Self::get(sdk, owner, spender);
        let new_allowance = current_allowance + amount;
        Self::set(sdk, owner, spender, new_allowance);
        Ok(())
    }
    fn subtract<SDK: SharedAPI>(
        sdk: &mut SDK,
        owner: Address,
        spender: Address,
        amount: U256,
    ) -> Result<(), &'static str> {
        let current_allowance = Self::get(sdk, owner, spender);
        if current_allowance < amount {
            return Err("insufficient allowance");
        }
        let new_allowance = current_allowance - amount;
        Self::set(sdk, owner, spender, new_allowance);
        Ok(())
    }
}

struct ERC20<SDK: SharedAPI> {
    sdk: SDK,
}

impl<SDK: SharedAPI> ERC20<SDK> {
    pub fn new(sdk: SDK) -> Self {
        ERC20 { sdk }
    }
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> ERC20API for ERC20<SDK> {
    fn symbol(&self) -> Bytes {
        Bytes::from("TOK")
    }

    fn name(&self) -> Bytes {
        Bytes::from("Token")
    }

    fn decimals(&self) -> U256 {
        U256::from(18)
    }

    fn total_supply(&self) -> U256 {
        U256::from_str_radix("1000000000000000000000000", 10).unwrap()
    }

    fn balance_of(&self, address: Address) -> U256 {
        Balance::get(&self.sdk, address)
    }

    fn transfer(&mut self, to: Address, value: U256) -> U256 {
        let from = self.sdk.context().contract_caller();

        // check if the sender and receiver are valid
        if from.is_zero() {
            panic!("invalid sender");
        } else if to.is_zero() {
            panic!("invalid receiver");
        }

        Balance::subtract(&mut self.sdk, from, value).unwrap_or_else(|err| panic!("{}", err));
        Balance::add(&mut self.sdk, to, value).unwrap_or_else(|err| panic!("{}", err));

        emit_event(&mut self.sdk, Transfer { from, to, value });
        U256::from(1)
    }

    fn allowance(&self, owner: Address, spender: Address) -> U256 {
        Allowance::get(&self.sdk, owner, spender)
    }

    fn approve(&mut self, spender: Address, value: U256) -> U256 {
        let owner = self.sdk.context().contract_caller();
        Allowance::set(&mut self.sdk, owner, spender, value);
        emit_event(
            &mut self.sdk,
            Approval {
                owner,
                spender,
                value,
            },
        );
        U256::from(1)
    }

    fn transfer_from(&mut self, from: Address, to: Address, value: U256) -> U256 {
        let spender = self.sdk.context().contract_caller();

        let current_allowance = Allowance::get(&self.sdk, from, spender);
        if current_allowance < value {
            panic!("insufficient allowance");
        }

        Allowance::subtract(&mut self.sdk, from, spender, value)
            .unwrap_or_else(|err| panic!("{}", err));
        Balance::subtract(&mut self.sdk, from, value).unwrap_or_else(|err| panic!("{}", err));
        Balance::add(&mut self.sdk, to, value).unwrap_or_else(|err| panic!("{}", err));

        emit_event(&mut self.sdk, Transfer { from, to, value });
        U256::from(1)
    }
}

impl<SDK: SharedAPI> ERC20<SDK> {
    pub fn deploy(&mut self) {
        let owner_address = self.sdk.context().contract_caller();
        let owner_balance: U256 = U256::from_str_radix("1000000000000000000000000", 10).unwrap();

        let _ = Balance::add(&mut self.sdk, owner_address, owner_balance);
    }
}

basic_entrypoint!(ERC20);

#[cfg(test)]
mod test {
    use super::*;
    use fluentbase_sdk::{
        address,
        journal::{JournalState, JournalStateBuilder},
        runtime::TestingContext,
        ContractContext,
    };
    use hex_literal::hex;
    use serial_test::serial;

    fn rewrite_input<T: Into<Vec<u8>>>(
        sdk: &mut JournalState<TestingContext>,
        input: T,
        caller: Option<Address>,
    ) {
        sdk.inner.borrow_mut().native_sdk.take_output();
        sdk.inner.borrow_mut().native_sdk.set_input(input);
        sdk.rewrite_contract_context(ContractContext {
            caller: caller.unwrap_or_default(),
            ..Default::default()
        });
    }

    fn with_test_input<T: Into<Vec<u8>>>(
        input: T,
        caller: Option<Address>,
    ) -> JournalState<TestingContext> {
        JournalStateBuilder::default()
            .with_contract_context(ContractContext {
                caller: caller.unwrap_or_default(),
                ..Default::default()
            })
            .with_devnet_genesis()
            .build(TestingContext::empty().with_input(input))
    }

    #[serial]
    #[test]
    pub fn test_deploy() {
        let owner_address = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));
        let owner_balance = U256::from_str_radix("1000000000000000000000000", 10).unwrap();

        // Set up the test input with the owner's address as the contract caller
        let sdk = with_test_input(vec![], Some(owner_address));
        let mut erc20 = ERC20::new(sdk);

        // Call the deployment function to initialize the contract state
        erc20.deploy();

        // Verify the balance
        let balance = Balance::get(&mut erc20.sdk, owner_address);
        assert_eq!(balance, owner_balance);
    }

    #[serial]
    #[test]
    pub fn test_name() {
        let call_name = nameCall {}.abi_encode();
        let expected_output = hex!("00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000005546f6b656e000000000000000000000000000000000000000000000000000000"); // "Token"
        let sdk = with_test_input(call_name, None);

        let mut erc20 = ERC20::new(sdk);
        erc20.deploy();
        erc20.main();

        let result = erc20.sdk.inner.borrow_mut().native_sdk.take_output();
        assert_eq!(result, expected_output.to_vec());
    }

    #[serial]
    #[test]
    pub fn test_symbol() {
        let call_symbol = symbolCall {}.abi_encode();
        let expected_output = hex!("00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000003544f4b0000000000000000000000000000000000000000000000000000000000"); // "TOK"

        let sdk = with_test_input(call_symbol, None);
        let mut erc20 = ERC20::new(sdk);
        erc20.deploy();
        erc20.main();

        let result = erc20.sdk.inner.borrow_mut().native_sdk.take_output();
        assert_eq!(result, expected_output.to_vec());
    }

    #[serial]
    #[test]
    pub fn test_balance_of() {
        let owner_address = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
        let expected_balance = "1000000000000000000000000";
        let sdk = with_test_input(vec![], Some(owner_address));
        let mut erc20 = ERC20::new(sdk);
        erc20.deploy();
        assert_eq!(
            Balance::get(&mut erc20.sdk, owner_address).to_string(),
            expected_balance
        );
        let call_balance_of =
            hex!("70a08231000000000000000000000000f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
        rewrite_input(&mut erc20.sdk, call_balance_of, Some(owner_address));
        erc20.main();
        let result = erc20.sdk.inner.borrow_mut().native_sdk.take_output();
        let output_balance = U256::from_be_slice(&result);
        assert_eq!(output_balance.to_string(), expected_balance);
    }

    #[serial]
    #[test]
    pub fn test_transfer() {
        let from = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
        let to = address!("390a4CEdBb65be7511D9E1a35b115376F39DbDF3");
        let value = U256::from_str_radix("100000000000000000000", 10).unwrap();
        let sdk = with_test_input(vec![], Some(from));
        let mut erc20 = ERC20::new(sdk);
        // run constructor
        erc20.deploy();
        // check balances
        // let balance_from = erc20.balances.get(from);
        assert_eq!(
            Balance::get(&mut erc20.sdk, from).to_string(),
            "1000000000000000000000000"
        );
        assert_eq!(Balance::get(&mut erc20.sdk, to).to_string(), "0");
        // transfer funds (100 tokens)
        rewrite_input(
            &mut erc20.sdk,
            transferCall { to, value }.abi_encode(),
            Some(from),
        );
        erc20.main();
        // check balances again
        assert_eq!(
            Balance::get(&mut erc20.sdk, from).to_string(),
            "999900000000000000000000"
        );
        assert_eq!(
            Balance::get(&mut erc20.sdk, to).to_string(),
            "100000000000000000000"
        );
    }

    #[serial]
    #[test]
    pub fn test_allowance() {
        let owner = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
        let spender = address!("390a4CEdBb65be7511D9E1a35b115376F39DbDF3");

        let approve_call = approveCall {
            spender,
            value: U256::from(1000),
        }
        .abi_encode();
        let sdk = with_test_input(approve_call, Some(owner));
        let mut erc20 = ERC20::new(sdk);

        // Approve allowance
        erc20.main();
        assert_eq!(Allowance::get(&erc20.sdk, owner, spender), U256::from(1000));

        // Check allowance
        let allowance_call = allowanceCall { owner, spender }.abi_encode();
        rewrite_input(&mut erc20.sdk, allowance_call, None);
        erc20.main();
        let result = erc20.sdk.inner.borrow_mut().native_sdk.take_output();
        let allowance = U256::from_be_slice(&result);
        assert_eq!(allowance, U256::from(1000));
    }

    #[serial]
    #[test]
    pub fn test_transfer_from() {
        let owner = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
        let spender = address!("390a4CEdBb65be7511D9E1a35b115376F39DbDF3");
        let recipient = address!("6dDb6e7F3b7e4991e3f75121aE3De2e1edE3bF19");

        let sdk = with_test_input(vec![], Some(owner));
        let mut erc20 = ERC20::new(sdk);

        // Deploy contract and approve allowance
        erc20.deploy();

        assert_eq!(
            Balance::get(&mut erc20.sdk, owner).to_string(),
            "1000000000000000000000000"
        );

        let approve_call = approveCall {
            spender,
            value: U256::from(1000),
        }
        .abi_encode();
        rewrite_input(&mut erc20.sdk, approve_call, Some(owner));
        erc20.main();

        // Transfer from owner to recipient via spender
        let transfer_from_call = transferFromCall {
            from: owner,
            to: recipient,
            value: U256::from(100),
        }
        .abi_encode();
        rewrite_input(&mut erc20.sdk, transfer_from_call, Some(spender));
        erc20.main();

        // Check balances and allowance
        assert_eq!(
            Balance::get(&mut erc20.sdk, owner).to_string(),
            "999999999999999999999900"
        );
        assert_eq!(Balance::get(&mut erc20.sdk, recipient).to_string(), "100");
        assert_eq!(
            Allowance::get(&mut erc20.sdk, owner, spender).to_string(),
            "900"
        );
    }
}
