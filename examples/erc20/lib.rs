use alloy_sol_types::SolEvent;
use fluentbase_sdk::{
    basic_entrypoint,
    derive::{router, Contract},
    AccountManager, Bytes, ContextReader, GuestContextReader, LowLevelSDK, SharedAPI, B256, U256,
};
use fluentbase_types::{contracts::EvmAPI, Address};
use hex_literal::hex;
use std::ptr;

pub trait ERC20API {
    fn name(&self) -> Bytes;
    fn symbol(&self) -> Bytes;
    fn decimals(&self) -> U256;
    fn total_supply(&self) -> U256;
    fn balance_of(&self, address: Address) -> U256;
    fn transfer(&mut self, to: Address, value: U256) -> U256;
}

sol! {
    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);
}

#[derive(Default)]
struct ERC20;

#[router(mode = "solidity")]
impl ERC20API for ERC20 {
    fn name<SDK: SharedAPI>(&self) -> Bytes {
        Bytes::from("Token")
    }
    fn symbol<SDK: SharedAPI>(&self) -> Bytes {
        Bytes::from("TOK")
    }
    fn decimals<SDK: SharedAPI>(&self) -> U256 {
        U256::from(18)
    }

    fn total_supply<SDK: SharedAPI>(&self) -> U256 {
        // TODO: d1r1 fix to use storage instead of hardcoded value
        U256::from_str_radix("1000000000000000000000000", 10).unwrap()
    }

    fn balance_of<SDK: SharedAPI>(&self, address: Address) -> U256 {
        let storage_key = storage_mapping_key(&STORAGE_BALANCES, address.abi_encode().as_slice());
        let storage_key_u256 = U256::from_be_slice(&storage_key);
        let cr = &fluentbase_sdk::GuestContextReader::DEFAULT;
        let am = &fluentbase_sdk::GuestAccountManager::DEFAULT;
        let contract_address = cr.contract_address();
        let (value, _is_cold) = am.storage(contract_address, storage_key_u256, false);
        let balance = U256::from_le_slice(value.as_le_slice());
        balance
    }
    fn transfer<SDK: SharedAPI>(&self, to: Address, value: U256) -> U256 {
        let cr = &fluentbase_sdk::GuestContextReader::DEFAULT;
        let am = &fluentbase_sdk::GuestAccountManager::DEFAULT;

        let contract_address = cr.contract_address();
        let from = cr.contract_caller();
        // check from/to addresses
        if from.is_zero() {
            panic!("invalid sender");
        } else if to.is_zero() {
            panic!("invalid receiver");
        }

        // update from balance
        {
            let from_balance_key =
                storage_mapping_key(&STORAGE_BALANCES, from.abi_encode().as_slice());
            let from_balance_key_u256 = U256::from_be_slice(&from_balance_key);
            let (from_balance_value, _is_cold) =
                am.storage(contract_address, from_balance_key_u256, false);
            let from_balance = U256::from_le_slice(from_balance_value.as_le_slice());
            if from_balance < value {
                panic!("insufficient balance");
            }
            let from_balance = from_balance - value;
            let success = am.write_storage(contract_address, from_balance_key_u256, from_balance);
            if !success {
                panic!("failed to update from balance");
            }
        }

        // update to balance
        {
            let to_balance_key = storage_mapping_key(&STORAGE_BALANCES, to.abi_encode().as_slice());
            let to_balance_key_u256 = U256::from_be_slice(&to_balance_key);
            let (to_balance_value, _is_cold) =
                am.storage(contract_address, to_balance_key_u256, false);
            let to_balance = U256::from_le_slice(to_balance_value.as_le_slice());
            let to_balance = to_balance + value;
            let success = am.write_storage(contract_address, to_balance_key_u256, to_balance);
            if !success {
                panic!("failed to update to balance");
            }
        }

        // emit event
        let transfer_event = Transfer {
            from: from.clone(),
            to,
            value,
        };

        let data: Bytes = transfer_event.encode_data().into();
        let topics: Vec<B256> = transfer_event
            .encode_topics()
            .iter()
            .map(|v| B256::from(v.0))
            .collect();

        am.log(contract_address, data, &topics);
        U256::from(1)
    }
}

impl ERC20 {
    pub fn deploy<SDK: SharedAPI>(&self) {
        let cr = &fluentbase_sdk::GuestContextReader::DEFAULT;
        let am = &fluentbase_sdk::GuestAccountManager::DEFAULT;

        let owner_address = cr.contract_caller();
        let owner_balance: U256 = U256::from_str_radix("1000000000000000000000000", 10).unwrap();

        // TODO: create macros that would be simplify working with storage
        // this structure should relay on the evm client
        // so we need to think about EVM CLIENT before implement the macros
        let storage_key =
            storage_mapping_key(&STORAGE_BALANCES, owner_address.abi_encode().as_slice());

        let storage_key_u256 = U256::from_be_slice(&storage_key);
        let owner_balance_u256 = U256::from_le_slice(owner_balance.as_le_slice());

        let contract_address = cr.contract_address();
        _ = am.write_storage(contract_address, storage_key_u256, owner_balance_u256);
    }
}

basic_entrypoint!(ERC20);

// ------------------------------------
// THIS IS A WORK IN PROGRESS SOLUTIONS
// ------------------------------------
const STORAGE_TOTAL_SUPPLY: [u8; 32] =
    hex!("0000000000000000000000000000000000000000000000000000000000000000");
const STORAGE_BALANCES: [u8; 32] =
    hex!("0000000000000000000000000000000000000000000000000000000000000001");
const STORAGE_ALLOWANCES: [u8; 32] =
    hex!("0000000000000000000000000000000000000000000000000000000000000002");
fn storage_mapping_key(slot: &[u8], value: &[u8]) -> [u8; 32] {
    let mut raw_storage_key: [u8; 64] = [0; 64];
    raw_storage_key[0..32].copy_from_slice(slot);
    raw_storage_key[32..64].copy_from_slice(value);
    let mut storage_key: [u8; 32] = [0; 32];
    LowLevelSDK::keccak256(
        raw_storage_key.as_ptr(),
        raw_storage_key.len() as u32,
        storage_key.as_mut_ptr(),
    );
    storage_key
}
// ------------------------------------

#[cfg(test)]
mod test {
    use super::*;
    use alloy_sol_types::{SolCall, SolType, SolValue};
    use fluentbase_sdk::codec::Encoder;
    use fluentbase_sdk::{Address, Bytes, ContractInput, LowLevelSDK, U256};
    use serial_test::serial;

    fn with_test_input<T: Into<Bytes>>(input: T, caller: Option<Address>) {
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
        let erc20 = ERC20::default();
        // Set up the test input with the owner's address as the contract caller
        with_test_input(vec![], Some(owner_address));
        let cr = &fluentbase_sdk::GuestContextReader::DEFAULT;
        let am = &fluentbase_sdk::GuestAccountManager::DEFAULT;

        // Call the deploy function to initialize the contract state
        erc20.deploy::<LowLevelSDK>();

        // Check if the owner's balance was correctly set
        let balance_key =
            storage_mapping_key(&STORAGE_BALANCES, owner_address.abi_encode().as_slice());
        let balance_key_u256 = U256::from_be_slice(&balance_key);

        // Get the balance from the storage
        let contract_address: Address = cr.contract_address();
        let (value, _is_cold) = am.storage(contract_address, balance_key_u256, false);

        // Verify the balance
        assert_eq!(value.to_string(), "1000000000000000000000000");
    }

    #[serial]
    #[test]
    pub fn test_name() {
        let call_name = nameCall {}.abi_encode();
        let expected_output = hex!("00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000005546f6b656e000000000000000000000000000000000000000000000000000000"); // "Token"

        with_test_input(call_name, None);

        let erc20 = ERC20::default();
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

        let erc20 = ERC20::default();
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
        let erc20 = ERC20::default();
        erc20.deploy::<LowLevelSDK>();

        let call_balance_of =
            hex!("70a08231000000000000000000000000f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
        with_test_input(call_balance_of, None);
        erc20.main::<LowLevelSDK>();

        let result = get_output();
        let output_balance = U256::from_be_slice(&result);
        assert_eq!(output_balance.to_string(), expected_balance);
    }

    fn get_balance(address: Address) -> U256 {
        let mut input = hex!("70a08231").to_vec();
        input.extend(address.abi_encode());
        let erc20 = ERC20::default();

        with_test_input(input, None);
        erc20.main::<LowLevelSDK>();

        let result = get_output();
        U256::abi_decode(&result, false).unwrap()
    }

    #[serial]
    #[test]
    pub fn test_transfer() {
        let from = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));
        let to = Address::from(hex!("390a4CEdBb65be7511D9E1a35b115376F39DbDF3"));
        let value = U256::from_str_radix("100000000000000000000", 10).unwrap();
        let erc20 = ERC20::default();

        // run constructor
        with_test_input(vec![], Some(from));
        erc20.deploy::<LowLevelSDK>();
        // check balances
        let balance_from = get_balance(from).to_string();
        assert_eq!(balance_from, "1000000000000000000000000");
        assert_eq!(get_balance(to).to_string(), "0");
        // transfer funds (100 tokens)
        with_test_input(transferCall { to, value }.abi_encode(), Some(from));
        erc20.main::<LowLevelSDK>();
        get_output();
        // check balances again
        assert_eq!(get_balance(from).to_string(), "999900000000000000000000");
        assert_eq!(get_balance(to).to_string(), "100000000000000000000");
    }
}
