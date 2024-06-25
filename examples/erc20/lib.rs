#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate fluentbase_sdk;

use alloc::{boxed::Box, vec::Vec};
use alloy_sol_types::SolEvent;
use fluentbase_sdk::{
    codec::Encoder,
    contracts::{EvmAPI, EvmClient, EvmSloadInput, EvmSstoreInput, PRECOMPILE_EVM},
    derive::{router, solidity_storage},
    AccountManager,
    Address,
    Bytes,
    ContextReader,
    LowLevelSDK,
    SharedAPI,
    B256,
    U256,
};
use hex_literal::hex;

pub trait ERC20API {
    fn name(&self) -> Bytes;
    fn symbol(&self) -> Bytes;
    fn decimals(&self) -> U256;
    fn total_supply(&self) -> U256;
    fn balance_of(&self, address: Address) -> U256;
    fn transfer(&self, to: Address, value: U256) -> U256;
    fn allowance(&self, owner: Address, spender: Address) -> U256;
    fn approve(&self, spender: Address, value: U256) -> U256;
    fn transfer_from(&self, from: Address, to: Address, value: U256) -> U256;
}

// Define the Transfer and Approval events
sol! {
    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);
}

fn emit_transfer_event<AM: AccountManager>(
    am: &AM,
    contract_address: Address,
    from: Address,
    to: Address,
    value: U256,
) {
    let transfer_event = Transfer { from, to, value };
    let data: Bytes = transfer_event.encode_data().into();
    let topics: Vec<B256> = transfer_event
        .encode_topics()
        .iter()
        .map(|v| B256::from(v.0))
        .collect();
    am.log(contract_address, data, &topics);
}

fn emit_approval_event<AM: AccountManager>(
    am: &AM,
    contract_address: Address,
    owner: Address,
    spender: Address,
    value: U256,
) {
    let approval_event = Approval {
        owner,
        spender,
        value,
    };
    let data: Bytes = approval_event.encode_data().into();
    let topics: Vec<B256> = approval_event
        .encode_topics()
        .iter()
        .map(|v| B256::from(v.0))
        .collect();
    am.log(contract_address, data, &topics);
}

solidity_storage! {
    mapping(Address => U256) Balance<EvmAPI>;
    mapping(Address => mapping(Address => U256)) Allowance<EvmAPI>;
}

impl<'a, T: EvmAPI> Balance<'a, T> {
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

impl<'a, T: EvmAPI> Allowance<'a, T> {
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

struct ERC20<'a, CR: ContextReader, AM: AccountManager, C: EvmAPI> {
    cr: &'a CR,
    am: &'a AM,

    balances: Balance<'a, C>,
    allowances: Allowance<'a, C>,
}

impl<'a, CR: ContextReader, AM: AccountManager, C: EvmAPI> ERC20<'a, CR, AM, C> {
    pub fn new(cr: &'a CR, am: &'a AM, client: &'a C) -> Self {
        ERC20 {
            cr,
            am,
            balances: Balance::new(client),
            allowances: Allowance::new(client),
        }
    }
}

#[router(mode = "solidity")]
impl<'a, CR: ContextReader, AM: AccountManager, C: EvmAPI> ERC20API for ERC20<'a, CR, AM, C> {
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
        let contract_address = self.cr.contract_address();
        let from = self.cr.contract_caller();

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

        emit_transfer_event(self.am, contract_address, from, to, value);
        U256::from(1)
    }

    fn allowance(&self, owner: Address, spender: Address) -> U256 {
        self.allowances.get(owner, spender)
    }
    fn approve(&self, spender: Address, value: U256) -> U256 {
        let owner = self.cr.contract_caller();

        self.allowances.set(owner, spender, value);

        emit_approval_event(self.am, self.cr.contract_address(), owner, spender, value);
        U256::from(1)
    }

    fn transfer_from(&self, from: Address, to: Address, value: U256) -> U256 {
        let spender = self.cr.contract_caller();

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

        emit_transfer_event(self.am, self.cr.contract_address(), from, to, value);
        U256::from(1)
    }
}

impl<'a, CR: ContextReader, AM: AccountManager, C: EvmAPI> ERC20<'a, CR, AM, C> {
    pub fn deploy<SDK: SharedAPI>(&self) {
        let owner_address = self.cr.contract_caller();
        let owner_balance: U256 = U256::from_str_radix("1000000000000000000000000", 10).unwrap();

        let _ = self.balances.add(owner_address, owner_balance);
    }
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
extern "C" fn deploy() {
    let cr = fluentbase_sdk::GuestContextReader::DEFAULT;
    let am = fluentbase_sdk::GuestAccountManager::DEFAULT;
    let evm_client = EvmClient::new(PRECOMPILE_EVM);
    let erc20 = ERC20::new(&cr, &am, &evm_client);
    erc20.deploy::<fluentbase_sdk::LowLevelSDK>();
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
extern "C" fn main() {
    let cr = fluentbase_sdk::GuestContextReader::DEFAULT;
    let am = fluentbase_sdk::GuestAccountManager::DEFAULT;
    let evm_client = EvmClient::new(PRECOMPILE_EVM);
    let erc20 = ERC20::new(&cr, &am, &evm_client);
    erc20.main::<fluentbase_sdk::LowLevelSDK>();
}

#[cfg(test)]
mod test {
    use super::*;
    use alloy_sol_types::SolCall;
    use fluentbase_sdk::{codec::Encoder, Address, Bytes, ContractInput, LowLevelSDK, U256};
    use serial_test::serial;

    fn with_test_input<T: Into<Bytes>>(input: T, caller: Option<Address>) {
        // Initalize genesis to be able to call system contracts (evm precompile)
        LowLevelSDK::init_with_devnet_genesis();

        let mut contract_input = ContractInput::default();
        contract_input.contract_caller = caller.unwrap_or_default();
        LowLevelSDK::with_test_context(contract_input.encode_to_vec(0));
        let input: Bytes = input.into();
        LowLevelSDK::with_test_input(input.into());
    }

    fn get_output() -> Vec<u8> {
        LowLevelSDK::get_test_output()
    }

    #[serial]
    #[test]
    pub fn test_deploy() {
        let owner_address = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));

        let owner_balance = U256::from_str_radix("1000000000000000000000000", 10).unwrap();

        let cr = fluentbase_sdk::GuestContextReader::DEFAULT;
        let am = fluentbase_sdk::GuestAccountManager::DEFAULT;
        let evm_client = EvmClient::new(PRECOMPILE_EVM);
        let erc20 = ERC20::new(&cr, &am, &evm_client);
        // Set up the test input with the owner's address as the contract caller
        with_test_input(vec![], Some(owner_address));

        // Call the deployment function to initialize the contract state

        erc20.deploy::<LowLevelSDK>();
        let balance = erc20.balances.get(owner_address);

        // Verify the balance
        assert_eq!(balance, owner_balance);
    }

    #[serial]
    #[test]
    pub fn test_name() {
        let call_name = nameCall {}.abi_encode();
        let expected_output = hex!("00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000005546f6b656e000000000000000000000000000000000000000000000000000000"); // "Token"

        with_test_input(call_name, None);

        let cr = fluentbase_sdk::GuestContextReader::DEFAULT;
        let am = fluentbase_sdk::GuestAccountManager::DEFAULT;
        let evm_client = EvmClient::new(PRECOMPILE_EVM);
        let erc20 = ERC20::new(&cr, &am, &evm_client);
        erc20.deploy::<LowLevelSDK>();
        erc20.main::<LowLevelSDK>();

        let output = get_output();
        assert_eq!(output, expected_output);
    }

    #[serial]
    #[test]
    pub fn test_symbol() {
        let call_symbol = symbolCall {}.abi_encode();
        let expected_output = hex!("00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000003544f4b0000000000000000000000000000000000000000000000000000000000"); // "TOK"

        with_test_input(call_symbol, None);

        let cr = fluentbase_sdk::GuestContextReader::DEFAULT;
        let am = fluentbase_sdk::GuestAccountManager::DEFAULT;
        let evm_client = EvmClient::new(PRECOMPILE_EVM);
        let erc20 = ERC20::new(&cr, &am, &evm_client);
        erc20.deploy::<LowLevelSDK>();
        erc20.main::<LowLevelSDK>();

        let output = get_output();

        assert_eq!(output, expected_output);
    }

    #[serial]
    #[test]
    pub fn test_balance_of() {
        let owner_address = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));
        let expected_balance = "1000000000000000000000000";

        with_test_input(vec![], Some(owner_address));
        let cr = fluentbase_sdk::GuestContextReader::DEFAULT;
        let am = fluentbase_sdk::GuestAccountManager::DEFAULT;
        let evm_client = EvmClient::new(PRECOMPILE_EVM);
        let erc20 = ERC20::new(&cr, &am, &evm_client);
        erc20.deploy::<LowLevelSDK>();

        let call_balance_of =
            hex!("70a08231000000000000000000000000f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
        with_test_input(call_balance_of, None);
        erc20.main::<LowLevelSDK>();

        let result = get_output();
        let output_balance = U256::from_be_slice(&result);
        assert_eq!(output_balance.to_string(), expected_balance);
    }

    #[serial]
    #[test]
    pub fn test_transfer() {
        let from = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));
        let to = Address::from(hex!("390a4CEdBb65be7511D9E1a35b115376F39DbDF3"));
        let value = U256::from_str_radix("100000000000000000000", 10).unwrap();
        let cr = fluentbase_sdk::GuestContextReader::DEFAULT;
        let am = fluentbase_sdk::GuestAccountManager::DEFAULT;
        let evm_client = EvmClient::new(PRECOMPILE_EVM);
        let erc20 = ERC20::new(&cr, &am, &evm_client);

        // run constructor
        with_test_input(vec![], Some(from));
        erc20.deploy::<LowLevelSDK>();
        // check balances
        // let balance_from = erc20.balances.get(from);
        assert_eq!(
            erc20.balances.get(from).to_string(),
            "1000000000000000000000000"
        );
        assert_eq!(erc20.balances.get(to).to_string(), "0");
        // transfer funds (100 tokens)
        with_test_input(transferCall { to, value }.abi_encode(), Some(from));
        erc20.main::<LowLevelSDK>();

        let _ = get_output();
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
        let owner = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));
        let spender = Address::from(hex!("390a4CEdBb65be7511D9E1a35b115376F39DbDF3"));
        let cr = fluentbase_sdk::GuestContextReader::DEFAULT;
        let am = fluentbase_sdk::GuestAccountManager::DEFAULT;
        let evm_client = EvmClient::new(PRECOMPILE_EVM);
        let erc20 = ERC20::new(&cr, &am, &evm_client);

        // Approve allowance
        let approve_call = approveCall {
            spender,
            value: U256::from(1000),
        }
        .abi_encode();

        with_test_input(approve_call, Some(owner));
        erc20.main::<LowLevelSDK>();

        let _ = get_output();

        // Check allowance
        let allowance_call = allowanceCall { owner, spender }.abi_encode();
        with_test_input(allowance_call, None);
        erc20.main::<LowLevelSDK>();

        let result = get_output();
        let allowance = U256::from_be_slice(&result);
        assert_eq!(allowance, U256::from(1000));
    }

    #[serial]
    #[test]
    pub fn test_transfer_from() {
        let owner = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));
        let spender = Address::from(hex!("390a4CEdBb65be7511D9E1a35b115376F39DbDF3"));
        let recipient = Address::from(hex!("6dDb6e7F3b7e4991e3f75121aE3De2e1edE3bF19"));
        let cr = fluentbase_sdk::GuestContextReader::DEFAULT;
        let am = fluentbase_sdk::GuestAccountManager::DEFAULT;
        let evm_client = EvmClient::new(PRECOMPILE_EVM);
        let erc20 = ERC20::new(&cr, &am, &evm_client);

        // Deploy contract and approve allowance
        with_test_input(vec![], Some(owner));
        erc20.deploy::<LowLevelSDK>();

        assert_eq!(
            erc20.balances.get(owner).to_string(),
            "1000000000000000000000000"
        );

        let approve_call = approveCall {
            spender,
            value: U256::from(1000),
        }
        .abi_encode();
        with_test_input(approve_call, Some(owner));
        erc20.main::<LowLevelSDK>();
        let _ = get_output();

        // Transfer from owner to recipient via spender
        let transfer_from_call = transferFromCall {
            from: owner,
            to: recipient,
            value: U256::from(100),
        }
        .abi_encode();
        with_test_input(transfer_from_call, Some(spender));
        erc20.main::<LowLevelSDK>();

        let _ = get_output();

        // Check balances and allowance
        assert_eq!(
            erc20.balances.get(owner).to_string(),
            "999999999999999999999900"
        );
        assert_eq!(erc20.balances.get(recipient).to_string(), "100");
        assert_eq!(erc20.allowances.get(owner, spender).to_string(), "900");
    }
}
