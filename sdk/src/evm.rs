use crate::{SysPlatformSDK, SDK};
use alloc::{vec, vec::Vec};
use alloy_primitives::{Address, B256, U256};
use alloy_sol_types::{sol, SolValue};
use byteorder::{BigEndian, ByteOrder};

sol! {
    struct ContractInput {
        bytes input;
        bytes bytecode;
        bytes32 hash;
        address address;
        address caller;
        uint256 value;
        bytes32 block_hash;
        uint256 balance;
        bytes env;
    }
}

#[derive(Debug, Copy, Clone)]
pub enum ContractError {
    InputDecodeFailure = -3001,
}

#[derive(Debug, Copy, Clone)]
enum ContractInputFields {
    InputOffset = 0,
    InputLength,
    BytecodeOffset,
    BytecodeLength,
    Hash,
    Address,
    Caller,
    Value,
    BlockHash,
    Balance,
    EnvOffset,
    EnvLength,
    _MaxFields,
}

const N_WORD_SIZE: usize = 32;

impl ContractInputFields {
    fn read_u256_word(self) -> [u8; N_WORD_SIZE] {
        let mut buffer: [u8; N_WORD_SIZE] = [0; N_WORD_SIZE];
        SDK::sys_read(buffer.as_mut_slice(), self as u32 * (N_WORD_SIZE as u32));
        buffer
    }

    fn input_size() -> u32 {
        Self::_MaxFields as u32 * (N_WORD_SIZE as u32)
    }
}

pub fn contract_read_input() -> Vec<u8> {
    // read offset and length
    let offset = ContractInputFields::InputOffset.read_u256_word();
    let offset = BigEndian::read_u32(&offset[28..]);
    let length = ContractInputFields::InputLength.read_u256_word();
    let length = BigEndian::read_u32(&length[28..]);
    // read input itself
    let mut buffer = vec![0; length as usize];
    SDK::sys_read(buffer.as_mut_slice(), offset);
    buffer
}

pub fn contract_read_bytecode() -> Vec<u8> {
    // read offset and length
    let offset = ContractInputFields::BytecodeOffset.read_u256_word();
    let offset = BigEndian::read_u32(&offset[28..]);
    let length = ContractInputFields::BytecodeLength.read_u256_word();
    let length = BigEndian::read_u32(&length[28..]);
    // read input itself
    let mut buffer = vec![0; length as usize];
    SDK::sys_read(buffer.as_mut_slice(), offset);
    buffer
}

pub fn contract_read_env() -> Vec<u8> {
    // read offset and length
    let offset = ContractInputFields::EnvOffset.read_u256_word();
    let offset = BigEndian::read_u32(&offset[28..]);
    let length = ContractInputFields::EnvLength.read_u256_word();
    let length = BigEndian::read_u32(&length[28..]);
    // read input itself
    let mut buffer = vec![0; length as usize];
    SDK::sys_read(buffer.as_mut_slice(), offset);
    buffer
}

pub fn contract_read_hash() -> B256 {
    let hash = ContractInputFields::Hash.read_u256_word();
    B256::from(hash)
}

pub fn contract_read_address() -> Address {
    let hash = ContractInputFields::Address.read_u256_word();
    Address::from_slice(&hash[12..])
}

pub fn contract_read_caller() -> Address {
    let hash = ContractInputFields::Caller.read_u256_word();
    Address::from_slice(&hash[12..])
}

pub fn contract_read_value() -> U256 {
    let hash = ContractInputFields::Value.read_u256_word();
    U256::from_be_slice(&hash[..])
}

pub fn contract_read_block_hash() -> B256 {
    let hash = ContractInputFields::BlockHash.read_u256_word();
    B256::from(hash)
}

pub fn contract_read_balance() -> U256 {
    let hash = ContractInputFields::Balance.read_u256_word();
    U256::from_be_slice(&hash[..])
}

#[cfg(feature = "runtime")]
impl ContractInput {
    pub fn encode(&self) -> Vec<u8> {
        let mut result = vec![];
        // encode ABI data
        let input_offset = ContractInputFields::input_size();
        let input_length = self.input.len() as u32;
        result.extend(&input_offset.abi_encode());
        result.extend(&input_length.abi_encode());
        let bytecode_offset = input_offset + input_length;
        let bytecode_length = self.bytecode.len() as u32;
        result.extend(&bytecode_offset.abi_encode());
        result.extend(&bytecode_length.abi_encode());
        result.extend(&self.hash.abi_encode());
        result.extend(&self.address.abi_encode());
        result.extend(&self.caller.abi_encode());
        result.extend(&self.value.abi_encode());
        result.extend(&self.block_hash.abi_encode());
        result.extend(&self.balance.abi_encode());
        let env_offset = bytecode_offset + bytecode_length;
        let env_length = self.env.len() as u32;
        result.extend(&env_offset.abi_encode());
        result.extend(&env_length.abi_encode());
        // encode raw data
        result.extend(&self.input);
        result.extend(&self.bytecode);
        result.extend(&self.env);
        result
    }
}

#[cfg(test)]
mod test {
    use crate::{
        evm::{contract_read_bytecode, contract_read_env, contract_read_input, ContractInput},
        SDK,
    };
    use alloc::vec;

    #[test]
    fn test_encode_decode() {
        // encode input and put into global var
        let contract_input = ContractInput {
            input: vec![0, 1, 2, 3],
            bytecode: vec![4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
            hash: Default::default(),
            address: Default::default(),
            caller: Default::default(),
            value: Default::default(),
            block_hash: Default::default(),
            balance: Default::default(),
            env: vec![10, 20, 30],
        };
        let encoded_input = contract_input.encode();
        // for chunk in encoded_input.chunks(32) {
        //     println!("{}", hex::encode(chunk));
        // }
        SDK::with_test_input(encoded_input);
        // read input fields
        let input = contract_read_input();
        assert_eq!(input, contract_input.input);
        let bytecode = contract_read_bytecode();
        assert_eq!(bytecode, contract_input.bytecode);
        let env = contract_read_env();
        assert_eq!(env, contract_input.env);
    }
}
