use alloy_sol_types::{sol, SolCall, SolType, SolValue};
use fluentbase_sdk::{
    evm::{Address, Bytes, U256},
    CryptoPlatformSDK,
    EvmPlatformSDK,
    SysPlatformSDK,
    SDK,
};
use hex_literal::hex;

sol! {
    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);

    function name() external view returns (string);
    function symbol() external view returns (string);
    function decimals() external view returns (uint8);

    function totalSupply() external view returns (uint256);
    function balanceOf(address account) external view returns (uint256);
    function transfer(address to, uint256 value) external returns (bool);
    function allowance(address owner, address spender) external view returns (uint256);
    function approve(address spender, uint256 value) external returns (bool);
    function transferFrom(address from, address to, uint256 value) external returns (bool);
}

const STORAGE_BALANCES: [u8; 32] =
    hex!("0000000000000000000000000000000000000000000000000000000000000000");
const STORAGE_ALLOWANCES: [u8; 32] =
    hex!("0000000000000000000000000000000000000000000000000000000000000001");

fn storage_mapping_key(slot: &[u8], value: &[u8]) -> [u8; 32] {
    let mut raw_storage_key: [u8; 64] = [0; 64];
    raw_storage_key[0..32].copy_from_slice(slot);
    raw_storage_key[32..64].copy_from_slice(value);
    let mut storage_key: [u8; 32] = [0; 32];
    SDK::crypto_keccak256(&raw_storage_key, &mut storage_key);
    storage_key
}

pub fn deploy() {
    let owner_address = SDK::evm_caller();
    let owner_balance: U256 = U256::from_str_radix("1000000000000000000000000", 10).unwrap();
    // mint balance to owner
    let storage_key = storage_mapping_key(&STORAGE_BALANCES, owner_address.abi_encode().as_slice());
    SDK::evm_sstore(&storage_key, owner_balance.as_le_slice())
}

struct ERC20;

impl ERC20 {
    fn name() -> Bytes {
        Bytes::from("Token")
    }

    fn symbol() -> Bytes {
        Bytes::from("TOK")
    }

    fn decimals() -> U256 {
        U256::from(18)
    }

    fn total_supply() -> U256 {
        U256::from(0)
    }

    fn balance_of(address: Address) -> U256 {
        let mut balance = U256::from(0);
        let storage_key = storage_mapping_key(&STORAGE_BALANCES, address.abi_encode().as_slice());
        unsafe {
            SDK::evm_sload(&storage_key, balance.as_le_slice_mut());
        }
        balance
    }

    fn transfer(to: Address, value: U256) -> U256 {
        // sender is a caller
        let from = SDK::evm_caller();
        // check from/to addresses
        if from.is_zero() {
            panic!("invalid sender");
        } else if to.is_zero() {
            panic!("invalid receiver");
        }
        // update from balance
        {
            let mut from_balance = U256::from(0);
            let from_balance_key =
                storage_mapping_key(&STORAGE_BALANCES, from.abi_encode().as_slice());
            unsafe {
                SDK::evm_sload(&from_balance_key, from_balance.as_le_slice_mut());
            }
            if from_balance < value {
                panic!("insufficient balance");
            }
            let from_balance = from_balance - value;
            SDK::evm_sstore(&from_balance_key, from_balance.as_le_slice());
        }
        // update to balance
        {
            let mut to_balance = U256::from(0);
            let to_balance_key = storage_mapping_key(&STORAGE_BALANCES, to.abi_encode().as_slice());
            unsafe {
                SDK::evm_sload(&to_balance_key, to_balance.as_le_slice_mut());
            }
            let to_balance = to_balance + value;
            SDK::evm_sstore(&to_balance_key, to_balance.as_le_slice());
        }
        U256::from(1)
    }
}

macro_rules! forward_evm_call {
    ($func_type:ty, $input:expr, $fn_name:expr, 0) => {{
        let output = $fn_name();
        SDK::sys_write(output.abi_encode().as_slice());
    }};
    ($func_type:ty, $input:expr, $fn_name:expr, 1) => {{
        let args_length =
            <<$func_type as SolCall>::Parameters<'_> as SolType>::ENCODED_SIZE.unwrap_or_default();
        SDK::sys_read(&mut $input[0..args_length], 4);
        let input =
            <<$func_type as SolCall>::Parameters<'_> as SolType>::abi_decode(&$input, false)
                .unwrap();
        let output = $fn_name(input.0);
        SDK::sys_write(output.abi_encode().as_slice());
    }};
    ($func_type:ty, $input:expr, $fn_name:expr, 2) => {{
        let args_length =
            <<$func_type as SolCall>::Parameters<'_> as SolType>::ENCODED_SIZE.unwrap_or_default();
        SDK::sys_read(&mut $input[0..args_length], 4);
        let input =
            <<$func_type as SolCall>::Parameters<'_> as SolType>::abi_decode(&$input, false)
                .unwrap();
        let output = $fn_name(input.0, input.1);
        SDK::sys_write(output.abi_encode().as_slice());
    }};
}

pub fn main() {
    let mut selector: [u8; 4] = [0; 4];
    SDK::sys_read(&mut selector, 0);
    // max number of inputs is 3 for ERC20 contract
    let mut input: [u8; 3 * 32] = [0; 3 * 32];
    match selector {
        nameCall::SELECTOR => forward_evm_call!(balanceOfCall, input, ERC20::name, 0),
        symbolCall::SELECTOR => forward_evm_call!(balanceOfCall, input, ERC20::symbol, 0),
        decimalsCall::SELECTOR => forward_evm_call!(balanceOfCall, input, ERC20::decimals, 0),
        totalSupplyCall::SELECTOR => {
            forward_evm_call!(balanceOfCall, input, ERC20::total_supply, 0)
        }
        balanceOfCall::SELECTOR => forward_evm_call!(balanceOfCall, input, ERC20::balance_of, 1),
        transferCall::SELECTOR => forward_evm_call!(transferCall, input, ERC20::transfer, 2),
        _ => panic!("unknown method"),
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use fluentbase_sdk::SDK;
    use hex_literal::hex;

    #[test]
    pub fn test_total_supply() {
        deploy();
        SDK::with_test_input(hex!("18160ddd").to_vec());
        main();
    }

    #[test]
    pub fn test_balance_of() {
        deploy();
        SDK::with_test_input(
            hex!("70a08231000000000000000000000000f39Fd6e51aad88F6F4ce6aB8827279cffFb92266")
                .to_vec(),
        );
        main();
        let result = SDK::get_test_output();
        assert_eq!(
            U256::from_le_slice(&result).to_string(),
            "1000000000000000000000000",
        );
    }

    fn get_balance(address: Address) -> U256 {
        let mut input = hex!("70a08231").to_vec();
        input.extend(address.abi_encode());
        SDK::with_test_input(input);
        main();
        let result = SDK::get_test_output();
        U256::abi_decode(&result, false).unwrap()
    }

    #[test]
    pub fn test_transfer() {
        let from = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));
        let to = Address::from(hex!("390a4CEdBb65be7511D9E1a35b115376F39DbDF3"));
        let value = U256::from_str_radix("100000000000000000000", 10).unwrap();
        // run constructor
        SDK::with_caller(from);
        deploy();
        // check balances
        assert_eq!(get_balance(from).to_string(), "1000000000000000000000000");
        assert_eq!(get_balance(to).to_string(), "0");
        // transfer funds (100 tokens)
        SDK::with_test_input(transferCall { to, value }.abi_encode());
        main();
        SDK::get_test_output();
        // check balances again
        assert_eq!(get_balance(from).to_string(), "999900000000000000000000");
        assert_eq!(get_balance(to).to_string(), "100000000000000000000");
    }
}
