use crate::{
    account::Account,
    evm::{
        address::_evm_address,
        balance::_evm_balance,
        calc_create2_address,
        calc_create_address,
        call::_evm_call,
        create::_evm_create,
        create2::_evm_create2,
        selfbalance::_evm_self_balance,
    },
    testing_utils::{generate_address_original_impl, TestingContext},
};
use alloc::{vec, vec::Vec};
use fluentbase_sdk::{evm::Address, Bytes20, Bytes32, LowLevelAPI, LowLevelSDK};
use fluentbase_types::{address, Bytes, B256, U256};
use keccak_hash::keccak;
use revm_interpreter::primitives::{alloy_primitives, hex, Bytecode};

// used contract:
// // SPDX-License-Identifier: MIT
// pragma solidity 0.8.24;
//
// contract HelloWorld {
//     function sayHelloWorld() public pure returns (string memory) {
//         return "Hello World";
//     }
//     function getBalanceAsStr(address addr) public view  returns (string memory) {
//         uint256 balance = addr.balance;
//         return toString(balance);
//     }
//     function getSelfBalanceAsStr() public view  returns (string memory) {
//         uint256 balance = address(this).balance;
//         return toString(balance);
//     }
//     function toString(uint256 value) internal pure returns (string memory) {
//         if (value == 0) {
//             return "0";
//         }
//
//         uint256 temp = value;
//         uint256 digits;
//
//         while (temp != 0) {
//             digits++;
//             temp /= 10;
//         }
//
//         bytes memory buffer = new bytes(digits);
//
//         while (value != 0) {
//             digits--;
//             buffer[digits] = bytes1(uint8(48 + (value % 10)));
//             value /= 10;
//         }
//
//         return string(buffer);
//     }
// }
// methods:
// {
//     "3b2e9748": "getBalanceAsStr(address)",
//     "48b8bcc3": "getSelfBalanceAsStr()",
//     "45773e4e": "sayHelloWorld()"
// }

static CONTRACT_BYTECODE: &[u8] = hex!("608060405234801561000f575f80fd5b506105ba8061001d5f395ff3fe608060405260043610610033575f3560e01c80633b2e97481461003757806345773e4e1461007357806348b8bcc31461009d575b5f80fd5b348015610042575f80fd5b5061005d600480360381019061005891906102f1565b6100bb565b60405161006a91906103a6565b60405180910390f35b34801561007e575f80fd5b506100876100e9565b60405161009491906103a6565b60405180910390f35b6100a5610126565b6040516100b291906103a6565b60405180910390f35b60605f8273ffffffffffffffffffffffffffffffffffffffff163190506100e18161013b565b915050919050565b60606040518060400160405280600b81526020017f48656c6c6f20576f726c64000000000000000000000000000000000000000000815250905090565b60605f4790506101358161013b565b91505090565b60605f8203610181576040518060400160405280600181526020017f3000000000000000000000000000000000000000000000000000000000000000815250905061028e565b5f8290505f5b5f82146101b0578080610199906103fc565b915050600a826101a99190610470565b9150610187565b5f8167ffffffffffffffff8111156101cb576101ca6104a0565b5b6040519080825280601f01601f1916602001820160405280156101fd5781602001600182028036833780820191505090505b5090505b5f8514610287578180610213906104cd565b925050600a8561022391906104f4565b603061022f9190610524565b60f81b81838151811061024557610244610557565b5b60200101907effffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff191690815f1a905350600a856102809190610470565b9450610201565b8093505050505b919050565b5f80fd5b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f6102c082610297565b9050919050565b6102d0816102b6565b81146102da575f80fd5b50565b5f813590506102eb816102c7565b92915050565b5f6020828403121561030657610305610293565b5b5f610313848285016102dd565b91505092915050565b5f81519050919050565b5f82825260208201905092915050565b5f5b83811015610353578082015181840152602081019050610338565b5f8484015250505050565b5f601f19601f8301169050919050565b5f6103788261031c565b6103828185610326565b9350610392818560208601610336565b61039b8161035e565b840191505092915050565b5f6020820190508181035f8301526103be818461036e565b905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f819050919050565b5f610406826103f3565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8203610438576104376103c6565b5b600182019050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601260045260245ffd5b5f61047a826103f3565b9150610485836103f3565b92508261049557610494610443565b5b828204905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52604160045260245ffd5b5f6104d7826103f3565b91505f82036104e9576104e86103c6565b5b600182039050919050565b5f6104fe826103f3565b9150610509836103f3565b92508261051957610518610443565b5b828206905092915050565b5f61052e826103f3565b9150610539836103f3565b9250828201905080821115610551576105506103c6565b5b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffdfea26469706673582212207ec3d35dc961bb0849482ddfc6287c3ebf1f4f3984b4bf9e55e8492f041fb2f164736f6c63430008180033").as_slice();
static CONTRACT_BYTECODE_METHOD_GET_BALANCE_AS_STR_ID: [u8; 4] = hex!("3b2e9748");
static CONTRACT_BYTECODE_METHOD_GET_SELF_BALANCE_AS_STR_ID: [u8; 4] = hex!("48b8bcc3");
static CONTRACT_BYTECODE_METHOD_SAY_HELLO_WORLD_ID: [u8; 4] = hex!("45773e4e");

#[test]
fn calc_create_address_test() {
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
        let mut init_code_hash: B256 = B256::default();
        LowLevelSDK::crypto_keccak256(
            init_code.as_ptr(),
            init_code.len() as u32,
            init_code_hash.as_mut_ptr(),
        );

        let expected = expected.parse::<Address>().unwrap();

        assert_eq!(expected, from.create2(salt, init_code_hash));
        assert_eq!(expected, from.create2_from_code(salt, init_code));
    }
}

#[test]
fn _evm_create_test() {
    let caller_address = address!("000000000000000000000000000000000000000c");
    let caller_nonce = 1;
    let caller_account = Account {
        address: caller_address,
        nonce: caller_nonce,
        balance: U256::from_be_slice(1000000000u128.to_be_bytes().as_slice()),
        ..Default::default()
    };

    let expected_contract_address = calc_create_address(&caller_address, caller_nonce);
    let block_hash = B256::left_padding_from(&hex!("0123456789abcdef"));
    let contract_value = U256::from_be_slice(&hex!("0123456789abcdef"));
    let contract_is_static = false;
    let block_coinbase: Address = address!("0000000000000000000000000000000000000012");
    let env_chain_id = 1;

    let contract_input_data_bytes = "some contract input".as_bytes();

    let mut test_ctx = TestingContext::new(true);
    test_ctx
        .try_add_account(&caller_account)
        .contract_input_wrapper
        .set_journal_checkpoint(LowLevelSDK::jzkt_checkpoint().into())
        .set_contract_input(Bytes::copy_from_slice(contract_input_data_bytes))
        .set_contract_input_size(contract_input_data_bytes.len() as u32)
        .set_env_chain_id(env_chain_id)
        .set_contract_address(expected_contract_address)
        .set_contract_caller(caller_address)
        .set_contract_bytecode(Bytes::copy_from_slice(CONTRACT_BYTECODE))
        .set_contract_code_size(CONTRACT_BYTECODE.len() as u32)
        .set_contract_code_hash(B256::from_slice(keccak(CONTRACT_BYTECODE).as_bytes()))
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
        CONTRACT_BYTECODE.as_ptr(),
        CONTRACT_BYTECODE.len() as u32,
        created_address.0.as_mut_ptr(),
        gas_limit,
    );
    assert!(exit_code.is_ok());
    assert_eq!(expected_contract_address, created_address);
}

#[test]
fn _evm_call_after_create_test() {
    let caller_address = address!("000000000000000000000000000000000000000c");
    let caller_nonce = 1;
    let caller_account = Account {
        address: caller_address,
        nonce: caller_nonce,
        balance: U256::from_be_slice(1000000000u128.to_be_bytes().as_slice()),
        ..Default::default()
    };

    let computed_contract_address = calc_create_address(&caller_address, caller_nonce);
    let block_hash = B256::left_padding_from(&hex!("0123456789abcdef"));
    let contract_value = U256::from_be_slice(&hex!("0123456789abcdef"));
    let contract_is_static = false;
    let block_coinbase: Address = address!("0000000000000000000000000000000000000012");
    let env_chain_id = 1;

    let contract_input_data_bytes = "some contract input".as_bytes();

    let mut test_ctx = TestingContext::new(true);
    test_ctx
        .try_add_account(&caller_account)
        .contract_input_wrapper
        .set_contract_input(Bytes::copy_from_slice(contract_input_data_bytes))
        .set_contract_input_size(contract_input_data_bytes.len() as u32)
        .set_env_chain_id(env_chain_id)
        .set_contract_address(computed_contract_address)
        .set_contract_caller(caller_address)
        .set_contract_bytecode(Bytes::copy_from_slice(CONTRACT_BYTECODE))
        .set_contract_code_size(CONTRACT_BYTECODE.len() as u32)
        .set_contract_code_hash(B256::from_slice(keccak(CONTRACT_BYTECODE).as_bytes()))
        .set_contract_value(contract_value)
        .set_block_hash(block_hash)
        .set_block_coinbase(block_coinbase)
        .set_tx_caller(caller_address)
        .set_contract_is_static(contract_is_static);
    test_ctx.apply_ctx();

    let create_value = U256::from_be_slice(&hex!("1000"));
    let gas_limit: u32 = 10_000_000;
    let mut created_address = Address::default();
    let exit_code = _evm_create(
        create_value.to_be_bytes::<32>().as_ptr(),
        CONTRACT_BYTECODE.as_ptr(),
        CONTRACT_BYTECODE.len() as u32,
        created_address.0.as_mut_ptr(),
        gas_limit,
    );
    assert!(exit_code.is_ok());
    assert_eq!(computed_contract_address, created_address);

    let mut args_data = Vec::from(CONTRACT_BYTECODE_METHOD_SAY_HELLO_WORLD_ID);
    let mut return_data: Vec<u8> = vec![0; 96];
    let call_value = U256::from_be_slice(&hex!("00"));
    let exit_code = _evm_call(
        gas_limit,
        created_address.as_ptr(),
        call_value.to_be_bytes::<32>().as_ptr(),
        args_data.as_ptr(),
        args_data.len() as u32,
        return_data.as_mut_ptr(),
        return_data.len() as u32,
    );
    assert!(exit_code.is_ok());
}

#[test]
fn _evm_call_after_create2_test() {
    let caller_address = address!("000000000000000000000000000000000000000c");
    let caller_nonce = 1;
    let caller_account = Account {
        address: caller_address,
        nonce: caller_nonce,
        balance: U256::from_be_slice(1000000000u128.to_be_bytes().as_slice()),
        ..Default::default()
    };

    let contract_bytecode_ =
        Bytecode::new_raw(alloy_primitives::Bytes::copy_from_slice(CONTRACT_BYTECODE));
    let contract_bytecode_hash = B256::from_slice(contract_bytecode_.hash_slow().as_slice());
    let salt = B256::left_padding_from(hex!("bc162382638a").as_slice());
    let computed_contract_address =
        calc_create2_address(&caller_address, &salt, &contract_bytecode_hash);
    let block_hash = B256::left_padding_from(&hex!("0123456789abcdef"));
    let contract_value = U256::from_be_slice(&hex!("0123456789abcdef"));
    let contract_is_static = false;
    let block_coinbase: Address = address!("0000000000000000000000000000000000000012");
    let env_chain_id = 1;

    let contract_input_data_bytes = "some contract input".as_bytes();

    let mut test_ctx = TestingContext::new(true);
    test_ctx
        .try_add_account(&caller_account)
        .contract_input_wrapper
        .set_contract_input(Bytes::copy_from_slice(contract_input_data_bytes))
        .set_contract_input_size(contract_input_data_bytes.len() as u32)
        .set_env_chain_id(env_chain_id)
        .set_contract_address(computed_contract_address)
        .set_contract_caller(caller_address)
        .set_contract_bytecode(Bytes::copy_from_slice(CONTRACT_BYTECODE))
        .set_contract_code_size(CONTRACT_BYTECODE.len() as u32)
        .set_contract_code_hash(contract_bytecode_hash)
        .set_contract_value(contract_value)
        .set_block_hash(block_hash)
        .set_block_coinbase(block_coinbase)
        .set_tx_caller(caller_address)
        .set_contract_is_static(contract_is_static);
    test_ctx.apply_ctx();

    let create_value = U256::from_be_slice(&hex!("1000"));
    let gas_limit: u32 = 10_000_000;
    let mut created_address = Address::default();
    let exit_code = _evm_create2(
        create_value.to_be_bytes::<32>().as_ptr(),
        CONTRACT_BYTECODE.as_ptr(),
        CONTRACT_BYTECODE.len() as u32,
        salt.as_ptr(),
        created_address.0.as_mut_ptr(),
        gas_limit,
    );
    assert!(exit_code.is_ok());
    assert_eq!(computed_contract_address, created_address);

    let mut args_data = Vec::from(CONTRACT_BYTECODE_METHOD_SAY_HELLO_WORLD_ID);
    let mut return_data: Vec<u8> = vec![0; 96];
    let call_value = U256::from_be_slice(&hex!("00"));
    let exit_code = _evm_call(
        gas_limit,
        created_address.as_ptr(),
        call_value.to_be_bytes::<32>().as_ptr(),
        args_data.as_ptr(),
        args_data.len() as u32,
        return_data.as_mut_ptr(),
        return_data.len() as u32,
    );
    assert!(exit_code.is_ok());
}

#[test]
fn _evm_balance_test() {
    let caller_address = address!("000000000000000000000000000000000000000c");
    let caller_nonce = 1;
    let caller_account = Account {
        address: caller_address,
        nonce: caller_nonce,
        balance: U256::from_be_slice(1234567u128.to_be_bytes().as_slice()),
        ..Default::default()
    };

    let expected_contract_address = calc_create_address(&caller_address, caller_nonce);
    let block_hash = B256::left_padding_from(&hex!("0123456789abcdef"));
    let contract_value = U256::from_be_slice(&hex!("0123456789abcdef"));
    let contract_is_static = false;
    let block_coinbase: Address = address!("0000000000000000000000000000000000000012");
    let env_chain_id = 1;

    let mut test_ctx = TestingContext::new(true);
    test_ctx
        .try_add_account(&caller_account)
        .contract_input_wrapper
        .set_env_chain_id(env_chain_id)
        .set_contract_address(expected_contract_address)
        .set_contract_caller(caller_address)
        .set_contract_value(contract_value)
        .set_block_hash(block_hash)
        .set_block_coinbase(block_coinbase)
        .set_tx_caller(caller_address)
        .set_contract_is_static(contract_is_static);
    test_ctx.apply_ctx();

    let mut caller_balance_bytes32_fact = Bytes32::default();
    _evm_balance(
        caller_address.as_ptr(),
        caller_balance_bytes32_fact.as_mut_ptr(),
    );
    let caller_balance_fact = U256::from_le_slice(caller_balance_bytes32_fact.as_slice());
    assert_eq!(caller_account.balance, caller_balance_fact);
}

#[test]
fn _evm_selfbalance_test() {
    let caller_address = address!("000000000000000000000000000000000000000c");
    let caller_nonce = 1;
    let caller_account = Account {
        address: caller_address,
        nonce: caller_nonce,
        balance: U256::from_be_slice(1234567u128.to_be_bytes().as_slice()),
        ..Default::default()
    };

    let block_hash = B256::left_padding_from(&hex!("0123456789abcdef"));
    let contract_value = U256::from_be_slice(&hex!("0123456789abcdef"));
    let contract_is_static = false;
    let block_coinbase: Address = address!("0000000000000000000000000000000000000012");
    let env_chain_id = 1;

    let mut test_ctx = TestingContext::new(true);
    test_ctx
        .try_add_account(&caller_account)
        .contract_input_wrapper
        .set_env_chain_id(env_chain_id)
        .set_contract_address(caller_address)
        .set_contract_caller(caller_address)
        .set_contract_value(contract_value)
        .set_block_hash(block_hash)
        .set_block_coinbase(block_coinbase)
        .set_tx_caller(caller_address)
        .set_contract_is_static(contract_is_static);
    test_ctx.apply_ctx();

    let mut caller_balance_bytes32_fact = Bytes32::default();
    _evm_self_balance(caller_balance_bytes32_fact.as_mut_ptr());
    let caller_balance_fact = U256::from_le_slice(caller_balance_bytes32_fact.as_slice());
    assert_eq!(caller_account.balance, caller_balance_fact);
}

#[test]
fn _evm_address_test() {
    let caller_address = address!("000000000000000000000000000000000000000c");
    let contract_address = address!("000000000000000000000000000000000000000b");
    let caller_nonce = 1;
    let caller_account = Account {
        address: caller_address,
        nonce: caller_nonce,
        balance: U256::from_be_slice(1234567u128.to_be_bytes().as_slice()),
        ..Default::default()
    };

    let block_hash = B256::left_padding_from(&hex!("0123456789abcdef"));
    let contract_value = U256::from_be_slice(&hex!("0123456789abcdef"));
    let contract_is_static = false;
    let block_coinbase: Address = address!("0000000000000000000000000000000000000012");
    let env_chain_id = 1;

    let mut test_ctx = TestingContext::new(true);
    test_ctx
        .try_add_account(&caller_account)
        .contract_input_wrapper
        .set_env_chain_id(env_chain_id)
        .set_contract_address(contract_address)
        .set_contract_caller(caller_address)
        .set_contract_value(contract_value)
        .set_block_hash(block_hash)
        .set_block_coinbase(block_coinbase)
        .set_tx_caller(caller_address)
        .set_contract_is_static(contract_is_static);
    test_ctx.apply_ctx();

    let mut address_bytes20_fact = Bytes20::default();
    _evm_address(address_bytes20_fact.as_mut_ptr());
    assert_eq!(contract_address.as_slice(), address_bytes20_fact.as_slice());
}

#[test]
fn _evm_selfbalance_from_contract_call_test() {
    let caller_address = address!("000000000000000000000000000000000000000c");
    let caller_nonce = 1;
    let caller_account = Account {
        address: caller_address,
        nonce: caller_nonce,
        balance: U256::from_be_slice(1000000000u128.to_be_bytes().as_slice()),
        ..Default::default()
    };

    let computed_contract_address = calc_create_address(&caller_address, caller_nonce);
    let block_hash = B256::left_padding_from(&hex!("0123456789abcdef"));
    let contract_value = U256::from_be_slice(&hex!("0123456789abcdef"));
    let contract_is_static = false;
    let block_coinbase: Address = address!("0000000000000000000000000000000000000012");
    let env_chain_id = 1;

    let contract_input_data_bytes = "some contract input".as_bytes();

    let mut test_ctx = TestingContext::new(true);
    test_ctx
        .try_add_account(&caller_account)
        .contract_input_wrapper
        .set_contract_input(Bytes::copy_from_slice(contract_input_data_bytes))
        .set_contract_input_size(contract_input_data_bytes.len() as u32)
        .set_env_chain_id(env_chain_id)
        .set_contract_address(computed_contract_address)
        .set_contract_caller(caller_address)
        .set_contract_bytecode(Bytes::copy_from_slice(CONTRACT_BYTECODE))
        .set_contract_code_size(CONTRACT_BYTECODE.len() as u32)
        .set_contract_code_hash(B256::from_slice(keccak(CONTRACT_BYTECODE).as_bytes()))
        .set_contract_value(contract_value)
        .set_block_hash(block_hash)
        .set_block_coinbase(block_coinbase)
        .set_tx_caller(caller_address)
        .set_contract_is_static(contract_is_static);
    test_ctx.apply_ctx();

    let create_value_hex_bytes = hex!("1000");
    let create_value = U256::from_be_slice(create_value_hex_bytes.as_slice());
    let gas_limit: u32 = 10_000_000;
    let mut created_address = Address::default();
    assert!(_evm_create(
        create_value.to_be_bytes::<32>().as_ptr(),
        CONTRACT_BYTECODE.as_ptr(),
        CONTRACT_BYTECODE.len() as u32,
        created_address.0.as_mut_ptr(),
        gas_limit,
    )
    .is_ok());
    assert_eq!(computed_contract_address, created_address);
    let mut created_address_balance = U256::default();
    Account::jzkt_get_balance(created_address.into_word().as_ptr(), unsafe {
        created_address_balance.as_le_slice_mut().as_mut_ptr()
    });
    assert_eq!(create_value, created_address_balance);

    let mut args_data = CONTRACT_BYTECODE_METHOD_GET_SELF_BALANCE_AS_STR_ID.to_vec();
    let mut return_data = [0u8; 96];
    let call_value = U256::from_be_slice(&hex!("00"));
    let exit_code = _evm_call(
        gas_limit,
        created_address.as_ptr(),
        call_value.to_be_bytes::<32>().as_ptr(),
        args_data.as_ptr(),
        args_data.len() as u32,
        return_data.as_mut_ptr(),
        return_data.len() as u32,
    );
    assert!(exit_code.is_ok());
    let mut expected_return_data = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 32, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 4, 52, 48, 57, 54, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0,
    ];
    assert_eq!(expected_return_data.as_slice(), return_data.as_slice());
}

#[test]
fn _evm_balance_from_contract_call_test() {
    let caller_address = address!("000000000000000000000000000000000000000c");
    let caller_nonce = 1;
    let caller_account = Account {
        address: caller_address,
        nonce: caller_nonce,
        balance: U256::from(432425321425u128),
        ..Default::default()
    };

    let computed_contract_address = calc_create_address(&caller_address, caller_nonce);
    let block_hash = B256::left_padding_from(&hex!("0123456789abcdef"));
    let contract_value = U256::from_be_slice(&hex!("0123456789abcdef"));
    let contract_is_static = false;
    let block_coinbase: Address = address!("0000000000000000000000000000000000000012");
    let env_chain_id = 1;

    let contract_input_data_bytes = "some contract input".as_bytes();

    let mut test_ctx = TestingContext::new(true);
    test_ctx
        .try_add_account(&caller_account)
        .contract_input_wrapper
        .set_contract_input(Bytes::copy_from_slice(contract_input_data_bytes))
        .set_contract_input_size(contract_input_data_bytes.len() as u32)
        .set_env_chain_id(env_chain_id)
        .set_contract_address(computed_contract_address)
        .set_contract_caller(caller_address)
        .set_contract_bytecode(Bytes::copy_from_slice(CONTRACT_BYTECODE))
        .set_contract_code_size(CONTRACT_BYTECODE.len() as u32)
        .set_contract_code_hash(B256::from_slice(keccak(CONTRACT_BYTECODE).as_bytes()))
        .set_contract_value(contract_value)
        .set_block_hash(block_hash)
        .set_block_coinbase(block_coinbase)
        .set_tx_caller(caller_address)
        .set_contract_is_static(contract_is_static);
    test_ctx.apply_ctx();

    let create_value_hex_bytes = hex!("84326482");
    let create_value = U256::from_be_slice(create_value_hex_bytes.as_slice());
    let gas_limit: u32 = 10_000_000;
    let mut created_address = Address::default();
    assert!(_evm_create(
        create_value.to_be_bytes::<32>().as_ptr(),
        CONTRACT_BYTECODE.as_ptr(),
        CONTRACT_BYTECODE.len() as u32,
        created_address.0.as_mut_ptr(),
        gas_limit,
    )
    .is_ok());
    assert_eq!(computed_contract_address, created_address);
    let mut created_address_balance = U256::default();
    Account::jzkt_get_balance(created_address.into_word().as_ptr(), unsafe {
        created_address_balance.as_le_slice_mut().as_mut_ptr()
    });
    assert_eq!(create_value, created_address_balance);

    let mut args_data = CONTRACT_BYTECODE_METHOD_GET_BALANCE_AS_STR_ID.to_vec();
    args_data.extend_from_slice(caller_address.into_word().as_slice());
    let mut return_data = [0u8; 96];
    let call_value = U256::from_be_slice(&hex!("00"));
    let exit_code = _evm_call(
        gas_limit,
        created_address.as_ptr(),
        call_value.to_be_bytes::<32>().as_ptr(),
        args_data.as_ptr(),
        args_data.len() as u32,
        return_data.as_mut_ptr(),
        return_data.len() as u32,
    );
    assert!(exit_code.is_ok());
    let mut expected_return_data = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 32, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 12, 52, 51, 48, 50, 48, 55, 52, 50, 54, 51, 56, 51, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    assert_eq!(expected_return_data.as_slice(), return_data.as_slice());
}
