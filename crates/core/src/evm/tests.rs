use crate::{
    account::Account,
    evm::{calc_create_address, call::_evm_call, create::_evm_create},
    testing_utils::{generate_address_original_impl, TestingContext},
};
use alloc::{vec, vec::Vec};
use fluentbase_sdk::{
    evm::{Address, JournalCheckpoint},
    LowLevelAPI,
    LowLevelSDK,
};
use fluentbase_types::{address, Bytes, B256, U256};
use keccak_hash::keccak;
use revm_interpreter::primitives::hex;

#[test]
fn create_address_correctness_test() {
    let tests = vec![(address!("0000000000000000000000000000000000000000"), 100)];
    for (address, nonce) in tests {
        assert_eq!(
            calc_create_address(&address, nonce),
            generate_address_original_impl(&address, nonce)
        )
    }
}

#[test]
fn create2_address_correctness_test() {
    let tests = [
        (
            "0000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "00",
            "4D1A2e2bB4F88F0250f26Ffff098B0b30B26BF38",
        ),
        (
            "deadbeef00000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "00",
            "B928f69Bb1D91Cd65274e3c79d8986362984fDA3",
        ),
        (
            "deadbeef00000000000000000000000000000000",
            "000000000000000000000000feed000000000000000000000000000000000000",
            "00",
            "D04116cDd17beBE565EB2422F2497E06cC1C9833",
        ),
        (
            "0000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "deadbeef",
            "70f2b2914A2a4b783FaEFb75f459A580616Fcb5e",
        ),
        (
            "00000000000000000000000000000000deadbeef",
            "00000000000000000000000000000000000000000000000000000000cafebabe",
            "deadbeef",
            "60f3f640a8508fC6a86d45DF051962668E1e8AC7",
        ),
        (
            "00000000000000000000000000000000deadbeef",
            "00000000000000000000000000000000000000000000000000000000cafebabe",
            "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
            "1d8bfDC5D46DC4f61D6b6115972536eBE6A8854C",
        ),
        (
            "0000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "",
            "E33C0C7F7df4809055C3ebA6c09CFe4BaF1BD9e0",
        ),
    ];
    for (from, salt, init_code, expected) in tests {
        let from = from.parse::<Address>().unwrap();

        let salt = hex::decode(salt).unwrap();
        let salt: [u8; 32] = salt.try_into().unwrap();

        let init_code = hex::decode(init_code).unwrap();
        let init_code_hash: B256 = keccak(&init_code).0.into();

        let expected = expected.parse::<Address>().unwrap();

        assert_eq!(expected, from.create2(salt, init_code_hash));
        assert_eq!(expected, from.create2_from_code(salt, init_code));
    }
}

#[test]
fn create_contract_test() {
    //
    let contract_bytecode = hex!("608060405234801561001057600080fd5b5061017c806100206000396000f3fe608060405234801561001057600080fd5b506004361061002b5760003560e01c806345773e4e14610030575b600080fd5b61003861004e565b6040516100459190610124565b60405180910390f35b60606040518060400160405280600b81526020017f48656c6c6f20576f726c64000000000000000000000000000000000000000000815250905090565b600081519050919050565b600082825260208201905092915050565b60005b838110156100c55780820151818401526020810190506100aa565b838111156100d4576000848401525b50505050565b6000601f19601f8301169050919050565b60006100f68261008b565b6101008185610096565b93506101108185602086016100a7565b610119816100da565b840191505092915050565b6000602082019050818103600083015261013e81846100eb565b90509291505056fea26469706673582212207ab46cb86e5d368ee5e146b9a6ebe9594ed3097882b30f23731b0558b704eb9d64736f6c634300080d0033").as_slice();

    let caller_address = address!("000000000000000000000000000000000000000c");
    let caller_nonce = 1;
    let expected_contract_address = calc_create_address(&caller_address, caller_nonce);
    let block_hash = B256::left_padding_from(&hex!("0123456789abcdef"));
    let contract_value = U256::from_be_slice(&hex!("0123456789abcdef"));
    let contract_is_static = false;
    let block_coinbase: Address = address!("0000000000000000000000000000000000000012");
    let env_chain_id = 1;

    let contract_input_data_str = "some contract input";

    let mut test_ctx = TestingContext::new();
    test_ctx
        .init_jzkt()
        .try_add_account(
            caller_address.clone(),
            Account {
                address: caller_address,
                code_size: 0,
                source_code_size: 0,
                nonce: caller_nonce,
                balance: U256::from_be_slice(1000000000u128.to_be_bytes().as_slice()),
                root: Default::default(),
                source_code_hash: Default::default(),
                code_hash: Default::default(),
            },
        )
        .contract_input_wrapper
        .set_journal_checkpoint(LowLevelSDK::jzkt_checkpoint().into())
        .set_contract_input(Bytes::copy_from_slice(contract_input_data_str.as_bytes()))
        .set_contract_input_size(contract_input_data_str.as_bytes().len() as u32)
        .set_env_chain_id(env_chain_id)
        .set_contract_address(expected_contract_address)
        .set_contract_caller(caller_address)
        .set_contract_bytecode(Bytes::copy_from_slice(contract_bytecode))
        .set_contract_code_size(contract_bytecode.len() as u32)
        .set_contract_code_hash(B256::from_slice(keccak(contract_bytecode).as_bytes()))
        .set_contract_value(contract_value)
        .set_block_hash(block_hash)
        .set_block_coinbase(block_coinbase)
        .set_tx_caller(caller_address)
        .set_contract_is_static(contract_is_static);
    test_ctx.apply_ctx();

    let value = B256::left_padding_from(&hex!("1000"));
    let gas_limit: u32 = 10_000_000;
    let mut created_address = Address::default();
    let exit_code = _evm_create(
        value.0.as_ptr(),
        contract_bytecode.as_ptr(),
        contract_bytecode.len() as u32,
        created_address.0.as_mut_ptr(),
        gas_limit,
    );
    assert!(exit_code.is_ok());
    assert_eq!(expected_contract_address, created_address);
}

#[test]
fn call_contract_test() {
    // // SPDX-License-Identifier: MIT
    // pragma solidity 0.8.13;
    //
    // contract HelloWorld {
    //     function sayHelloWorld() public pure returns (string memory) { # 0x45773E4E
    //         return "Hello World";
    //     }
    // }
    let contract_bytecode = hex!("608060405234801561001057600080fd5b5061017c806100206000396000f3fe608060405234801561001057600080fd5b506004361061002b5760003560e01c806345773e4e14610030575b600080fd5b61003861004e565b6040516100459190610124565b60405180910390f35b60606040518060400160405280600b81526020017f48656c6c6f20576f726c64000000000000000000000000000000000000000000815250905090565b600081519050919050565b600082825260208201905092915050565b60005b838110156100c55780820151818401526020810190506100aa565b838111156100d4576000848401525b50505050565b6000601f19601f8301169050919050565b60006100f68261008b565b6101008185610096565b93506101108185602086016100a7565b610119816100da565b840191505092915050565b6000602082019050818103600083015261013e81846100eb565b90509291505056fea26469706673582212207ab46cb86e5d368ee5e146b9a6ebe9594ed3097882b30f23731b0558b704eb9d64736f6c634300080d0033").as_slice();
    let say_hello_world_method_id = hex!("45773E4E").as_slice();

    let caller_address = address!("000000000000000000000000000000000000000c");
    let caller_nonce = 1;
    let expected_contract_address = calc_create_address(&caller_address, caller_nonce);
    let block_hash = B256::left_padding_from(&hex!("0123456789abcdef"));
    let contract_value = U256::from_be_slice(&hex!("0123456789abcdef"));
    let contract_is_static = false;
    let block_coinbase: Address = address!("0000000000000000000000000000000000000012");
    let env_chain_id = 1;

    let contract_input_data_str = "some contract input";

    let mut test_ctx = TestingContext::new();
    test_ctx
        .init_jzkt()
        .try_add_account(
            caller_address.clone(),
            Account {
                address: caller_address,
                code_size: 0,
                source_code_size: 0,
                nonce: caller_nonce,
                balance: U256::from_be_slice(1000000000u128.to_be_bytes().as_slice()),
                root: Default::default(),
                source_code_hash: Default::default(),
                code_hash: Default::default(),
            },
        )
        .contract_input_wrapper
        .set_contract_input(Bytes::copy_from_slice(contract_input_data_str.as_bytes()))
        .set_contract_input_size(contract_input_data_str.as_bytes().len() as u32)
        .set_env_chain_id(env_chain_id)
        .set_contract_address(expected_contract_address)
        .set_contract_caller(caller_address)
        .set_contract_bytecode(Bytes::copy_from_slice(contract_bytecode))
        .set_contract_code_size(contract_bytecode.len() as u32)
        .set_contract_code_hash(B256::from_slice(keccak(contract_bytecode).as_bytes()))
        .set_contract_value(contract_value)
        .set_block_hash(block_hash)
        .set_block_coinbase(block_coinbase)
        .set_tx_caller(caller_address)
        .set_contract_is_static(contract_is_static);
    test_ctx.apply_ctx();

    let value = B256::left_padding_from(&hex!("1000"));
    let gas_limit: u32 = 10_000_000;
    let mut created_address = Address::default();
    let exit_code = _evm_create(
        value.0.as_ptr(),
        contract_bytecode.as_ptr(),
        contract_bytecode.len() as u32,
        created_address.0.as_mut_ptr(),
        gas_limit,
    );
    assert!(exit_code.is_ok());
    assert_eq!(expected_contract_address, created_address);

    let mut args_data: &[u8] = say_hello_world_method_id; // method
    let mut return_data: Vec<u8> = vec![0; 96];
    let exit_code = _evm_call(
        gas_limit,
        created_address.as_ptr(),
        value.0.as_ptr(),
        args_data.as_ptr(),
        args_data.len() as u32,
        return_data.as_mut_ptr(),
        return_data.len() as u32,
    );
    assert!(exit_code.is_ok());
}
