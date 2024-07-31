#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate fluentbase_sdk;

use alloc::vec::Vec;
use alloy_sol_types::SolEvent;
use fluentbase_sdk::{
    contracts::{EvmAPI, EvmSloadInput, EvmSstoreInput},
    derive::{router, solidity_storage},
    Address,
    Bytes,
    B256,
    U256,
};
use fluentbase_types::SharedAPI;

pub trait ERC20API {
    fn name(&self) -> Bytes;
    fn symbol(&self) -> Bytes;
    fn decimals(&self) -> U256;
    fn total_supply(&self) -> U256;
    fn balance_of(&self, address: Address) -> U256;
    fn transfer(&self, to: Address, value: U256) -> U256;
    fn allowance(&self, owner: Address, spender: Address) -> U256;
    fn approve(&self, spender: Address, value: U256) -> U256;
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
    sdk.write_log(data, &topics);
}

solidity_storage! {
    mapping(Address => U256) Balance;
    mapping(Address => mapping(Address => U256)) Allowance;
}

impl<'a, SDK: SharedAPI, T: EvmAPI> Balance<'a, SDK, T> {
    fn add(&self, address: Address, amount: U256) -> Result<(), &'static str> {
        let current_balance = self.get(address);
        let new_balance = current_balance + amount;
        self.set(address, new_balance);
        Ok(())
    }
    fn subtract(&self, address: Address, amount: U256) -> Result<(), &'static str> {
        let current_balance = self.get(address);
        if current_balance < amount {
            return Err("insufficient balance");
        }
        let new_balance = current_balance - amount;
        self.set(address, new_balance);
        Ok(())
    }
}

impl<'a, SDK: SharedAPI, T: EvmAPI> Allowance<'a, SDK, T> {
    fn add(&self, owner: Address, spender: Address, amount: U256) -> Result<(), &'static str> {
        let current_allowance = self.get(owner, spender);
        let new_allowance = current_allowance + amount;
        self.set(owner, spender, new_allowance);
        Ok(())
    }
    fn subtract(&self, owner: Address, spender: Address, amount: U256) -> Result<(), &'static str> {
        let current_allowance = self.get(owner, spender);
        if current_allowance < amount {
            return Err("insufficient allowance");
        }
        let new_allowance = current_allowance - amount;
        self.set(owner, spender, new_allowance);
        Ok(())
    }
}

struct ERC20<'a, SDK: SharedAPI, C: EvmAPI> {
    sdk: SDK,
    balances: Balance<'a, SDK, C>,
    allowances: Allowance<'a, SDK, C>,
}

impl<'a, SDK: SharedAPI, C: EvmAPI> ERC20<'a, SDK, C> {
    pub fn new(sdk: SDK, client: &'a C) -> Self {
        ERC20 {
            sdk,
            balances: Balance::new(client),
            allowances: Allowance::new(client),
        }
    }
}

#[router(mode = "solidity")]
impl<'a, SDK: SharedAPI, C: EvmAPI> ERC20API for ERC20<'a, SDK, C> {
    fn name(&self) -> Bytes {
        Bytes::from("Token")
    }
    fn symbol(&self) -> Bytes {
        Bytes::from("TOK")
    }
    fn decimals(&self) -> U256 {
        U256::from(18)
    }

    fn total_supply(&self) -> U256 {
        U256::from_str_radix("1000000000000000000000000", 10).unwrap()
    }

    fn balance_of(&self, address: Address) -> U256 {
        self.balances.get(address)
    }

    fn transfer(&self, to: Address, value: U256) -> U256 {
        let contract_address = self.sdk.contract_address();
        let from = self.ctx.contract_caller();

        // check if the sender and receiver are valid
        if from.is_zero() {
            panic!("invalid sender");
        } else if to.is_zero() {
            panic!("invalid receiver");
        }

        self.balances
            .subtract(from, value)
            .unwrap_or_else(|err| panic!("{}", err));
        self.balances
            .add(to, value)
            .unwrap_or_else(|err| panic!("{}", err));

        emit_event(&self.sdk, Transfer { from, to, value });
        U256::from(1)
    }

    fn allowance(&self, owner: Address, spender: Address) -> U256 {
        self.allowances.get(owner, spender)
    }
    fn approve(&self, spender: Address, value: U256) -> U256 {
        let owner = self.ctx.contract_caller();

        self.allowances.set(owner, spender, value);

        emit_event(
            &self.sdk,
            Approval {
                owner,
                spender,
                value,
            },
        );
        U256::from(1)
    }

    fn transfer_from(&mut self, from: Address, to: Address, value: U256) -> U256 {
        let spender = self.sdk.contract_context().caller;

        let current_allowance = self.allowances.get(from, spender);

        if current_allowance < value {
            panic!("insufficient allowance");
        }

        self.allowances
            .subtract(from, spender, value)
            .unwrap_or_else(|err| panic!("{}", err));
        self.balances
            .subtract(from, value)
            .unwrap_or_else(|err| panic!("{}", err));
        self.balances
            .add(to, value)
            .unwrap_or_else(|err| panic!("{}", err));

        emit_event(&mut self.sdk, Transfer { from, to, value });
        U256::from(1)
    }
}

impl<'a, SDK: SharedAPI, C: EvmAPI> ERC20<'a, SDK, C> {
    pub fn deploy(&self) {
        let owner_address = self.sdk.contract_context().caller;
        let owner_balance: U256 = U256::from_str_radix("1000000000000000000000000", 10).unwrap();

        let _ = self.balances.add(owner_address, owner_balance);
    }
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
extern "C" fn deploy() {
    use fluentbase_sdk::{
        contracts::{EvmClient, PRECOMPILE_EVM},
        rwasm::{RwasmContext, RwasmContextReader},
    };
    let evm_client = EvmClient::new(RwasmContext {}, PRECOMPILE_EVM);
    let erc20 = ERC20::new(RwasmContextReader {}, RwasmContext {}, &evm_client);
    erc20.deploy();
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
extern "C" fn main() {
    use fluentbase_sdk::{
        contracts::{EvmClient, PRECOMPILE_EVM},
        rwasm::{RwasmContext, RwasmContextReader},
    };
    let evm_client = EvmClient::new(RwasmContext {}, PRECOMPILE_EVM);
    let erc20 = ERC20::new(RwasmContextReader {}, RwasmContext {}, &evm_client);
    erc20.main();
}

#[cfg(test)]
mod test {
    use super::*;
    use fluentbase_sdk::{codec::Encoder, contracts::EvmClient, runtime::TestingContext};
    use fluentbase_types::{address, contracts::PRECOMPILE_EVM};
    use hex_literal::hex;
    use serial_test::serial;

    fn with_test_input<T: Into<Bytes>>(input: T, caller: Option<Address>) -> (TestingContext) {
        let input: Bytes = input.into();
        let ctx = TestingContext::new().with_input(input.to_vec());
        // Initialize genesis to be able to call system contracts (evm precompile)
        let mut contract_input = ContractInput::default();
        contract_input.contract_caller = caller.unwrap_or_default();
        (
            ctx.with_context(contract_input.encode_to_vec(0)),
            contract_input,
        )
    }

    #[serial]
    #[test]
    pub fn test_deploy() {
        let owner_address = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));

        let owner_balance = U256::from_str_radix("1000000000000000000000000", 10).unwrap();

        let (sdk, ctx) = with_test_input(vec![], Some(owner_address));
        let evm_client = EvmClient::new(sdk.clone(), PRECOMPILE_EVM);
        let erc20 = ERC20::new(ctx, sdk, &evm_client);
        // Set up the test input with the owner's address as the contract caller

        // Call the deployment function to initialize the contract state

        erc20.deploy();
        let balance = erc20.balances.get(owner_address);

        // Verify the balance
        assert_eq!(balance, owner_balance);
    }

    #[serial]
    #[test]
    pub fn test_name() {
        let call_name = nameCall {}.abi_encode();
        let expected_output = hex!("00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000005546f6b656e000000000000000000000000000000000000000000000000000000"); // "Token"
        let (sdk, ctx) = with_test_input(call_name, None);

        let evm_client = EvmClient::new(sdk.clone(), PRECOMPILE_EVM);
        let erc20 = ERC20::new(ctx, sdk.clone(), &evm_client);
        erc20.deploy();
        erc20.main();

        assert_eq!(sdk.output(), expected_output);
    }

    #[serial]
    #[test]
    pub fn test_symbol() {
        let call_symbol = symbolCall {}.abi_encode();
        let expected_output = hex!("00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000003544f4b0000000000000000000000000000000000000000000000000000000000"); // "TOK"

        let (sdk, ctx) = with_test_input(call_symbol, None);

        let evm_client = EvmClient::new(sdk.clone(), PRECOMPILE_EVM);
        let erc20 = ERC20::new(ctx, sdk.clone(), &evm_client);
        erc20.deploy();
        erc20.main();

        assert_eq!(sdk.output(), expected_output);
    }

    #[serial]
    #[test]
    pub fn test_balance_of() {
        let owner_address = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
        let expected_balance = "1000000000000000000000000";
        let (sdk, ctx) = with_test_input(vec![], Some(owner_address));
        let evm_client = EvmClient::new(sdk.clone(), PRECOMPILE_EVM);
        let erc20 = ERC20::new(ctx, sdk.clone(), &evm_client);
        erc20.deploy();
        let call_balance_of =
            hex!("70a08231000000000000000000000000f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
        let (sdk, ctx) = with_test_input(call_balance_of, None);
        let erc20 = ERC20::new(ctx, sdk.clone(), &evm_client);
        erc20.main();
        let result = sdk.output();
        let output_balance = U256::from_be_slice(&result);
        assert_eq!(output_balance.to_string(), expected_balance);
    }

    #[serial]
    #[test]
    pub fn test_transfer() {
        let from = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
        let to = address!("390a4CEdBb65be7511D9E1a35b115376F39DbDF3");
        let value = U256::from_str_radix("100000000000000000000", 10).unwrap();
        let (sdk, ctx) = with_test_input(vec![], Some(from));
        let evm_client = EvmClient::new(sdk.clone(), PRECOMPILE_EVM);
        let erc20 = ERC20::new(ctx, sdk.clone(), &evm_client);
        // run constructor
        erc20.deploy();
        // check balances
        // let balance_from = erc20.balances.get(from);
        assert_eq!(
            erc20.balances.get(from).to_string(),
            "1000000000000000000000000"
        );
        assert_eq!(erc20.balances.get(to).to_string(), "0");
        // transfer funds (100 tokens)
        let (sdk, ctx) = with_test_input(transferCall { to, value }.abi_encode(), Some(from));
        let erc20 = ERC20::new(ctx, sdk.clone(), &evm_client);
        erc20.main();
        // check balances again
        assert_eq!(
            erc20.balances.get(from).to_string(),
            "999900000000000000000000"
        );
        assert_eq!(erc20.balances.get(to).to_string(), "100000000000000000000");
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
        let (sdk, ctx) = with_test_input(approve_call, Some(owner));

        let evm_client = EvmClient::new(sdk.clone(), PRECOMPILE_EVM);
        let erc20 = ERC20::new(ctx, sdk.clone(), &evm_client);

        // Approve allowance
        erc20.main();

        // Check allowance
        let allowance_call = allowanceCall { owner, spender }.abi_encode();
        let (sdk, ctx) = with_test_input(allowance_call, None);
        let erc20 = ERC20::new(ctx, sdk.clone(), &evm_client);
        erc20.main();
        let result = sdk.output();
        let allowance = U256::from_be_slice(&result);
        assert_eq!(allowance, U256::from(1000));
    }

    #[serial]
    #[test]
    pub fn test_transfer_from() {
        let owner = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
        let spender = address!("390a4CEdBb65be7511D9E1a35b115376F39DbDF3");
        let recipient = address!("6dDb6e7F3b7e4991e3f75121aE3De2e1edE3bF19");

        let (sdk, ctx) = with_test_input(vec![], Some(owner));

        let evm_client = EvmClient::new(sdk.clone(), PRECOMPILE_EVM);
        let erc20 = ERC20::new(ctx, sdk.clone(), &evm_client);

        // Deploy contract and approve allowance
        erc20.deploy();

        assert_eq!(
            erc20.balances.get(owner).to_string(),
            "1000000000000000000000000"
        );

        let approve_call = approveCall {
            spender,
            value: U256::from(1000),
        }
        .abi_encode();
        let (sdk, ctx) = with_test_input(approve_call, Some(owner));
        let evm_client = EvmClient::new(sdk.clone(), PRECOMPILE_EVM);
        let erc20 = ERC20::new(ctx, sdk.clone(), &evm_client);
        erc20.main();

        // Transfer from owner to recipient via spender
        let transfer_from_call = transferFromCall {
            from: owner,
            to: recipient,
            value: U256::from(100),
        }
        .abi_encode();
        let (sdk, ctx) = with_test_input(transfer_from_call, Some(spender));
        erc20.main();

        // Check balances and allowance
        assert_eq!(
            erc20.balances.get(owner).to_string(),
            "999999999999999999999900"
        );
        assert_eq!(erc20.balances.get(recipient).to_string(), "100");
        assert_eq!(erc20.allowances.get(owner, spender).to_string(), "900");
    }
}
