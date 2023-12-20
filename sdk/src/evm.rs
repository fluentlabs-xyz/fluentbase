use crate::{SysPlatformSDK, SDK};
use alloc::{vec, vec::Vec};
pub use alloy_primitives::{Address, Bytes, B256, U256};
use fluentbase_codec::define_codec_struct;

define_codec_struct! {
    pub struct ContractInput {
        // env info
        env_chain_id: u64,
        // contract info
        contract_address: Address,
        contract_caller: Address,
        contract_bytecode: Vec<u8>,
        contract_code_size: u32,
        contract_code_hash: B256,
        contract_input: Vec<u8>,
        contract_value: U256,
        // block info
        block_hash: B256,
        block_coinbase: Address,
        block_timestamp: u64,
        block_number: u64,
        block_difficulty: u64,
        block_gas_limit: u64,
        block_base_fee: U256,
        // tx info
        tx_gas_priority_fee: Option<U256>,
        tx_caller: Address,
        tx_blob_hashes: Vec<B256>,
        tx_blob_gas_price: u64,
    }
}

macro_rules! impl_reader_helper {
    ($input_type:ty, $return_typ:ty) => {{
        let mut buffer: [u8; <$input_type>::FIELD_SIZE] = [0; <$input_type>::FIELD_SIZE];
        SDK::sys_read(&mut buffer, <$input_type>::FIELD_OFFSET as u32);
        let mut result: $return_typ = Default::default();
        let (offset, length) = <$input_type>::decode_field_header_at(&buffer, 0, &mut result);
        if length > 0 {
            let mut buffer2 = vec![0; offset + length];
            buffer2[0..<$input_type>::FIELD_SIZE].copy_from_slice(&buffer);
            SDK::sys_read(
                &mut buffer2.as_mut_slice()[offset..(offset + length)],
                offset as u32,
            );
            <$input_type>::decode_field_body_at(&buffer2, 0, &mut result);
        }
        result
    }};
}
macro_rules! impl_reader_func {
    (fn $fn_name:ident() -> $return_typ:ty, $input_type:ty) => {
        pub fn $fn_name() -> $return_typ {
            impl_reader_helper!($input_type, $return_typ)
        }
    };
}

#[derive(Default)]
pub struct ExecutionContext;

impl ExecutionContext {
    // env info
    impl_reader_func!(fn env_chain_id() -> u64, ContractInput::EnvChainId);
    // contract info
    impl_reader_func!(fn contract_address() -> Address, ContractInput::ContractAddress);
    impl_reader_func!(fn contract_caller() -> Address, ContractInput::ContractCaller);
    impl_reader_func!(fn contract_bytecode() -> Vec<u8>, ContractInput::ContractBytecode);
    impl_reader_func!(fn contract_code_size() -> u32, ContractInput::ContractCodeSize);
    impl_reader_func!(fn contract_code_hash() -> B256, ContractInput::ContractCodeHash);
    impl_reader_func!(fn contract_input() -> Vec<u8>, ContractInput::ContractInput);
    impl_reader_func!(fn contract_value() -> U256, ContractInput::ContractValue);
    // block info
    impl_reader_func!(fn block_hash() -> B256, ContractInput::BlockHash);
    impl_reader_func!(fn block_coinbase() -> Address, ContractInput::BlockCoinbase);
    impl_reader_func!(fn block_timestamp() -> u64, ContractInput::BlockTimestamp);
    impl_reader_func!(fn block_number() -> u64, ContractInput::BlockNumber);
    impl_reader_func!(fn block_difficulty() -> u64, ContractInput::BlockDifficulty);
    impl_reader_func!(fn block_gas_limit() -> u64, ContractInput::BlockGasLimit);
    impl_reader_func!(fn block_base_fee() -> U256, ContractInput::BlockBaseFee);
    // tx info
    impl_reader_func!(fn tx_gas_priority_fee() -> Option<U256>, ContractInput::TxGasPriorityFee);
    impl_reader_func!(fn tx_caller() -> Address, ContractInput::TxCaller);
    impl_reader_func!(fn tx_blob_hashes() -> Vec<B256>, ContractInput::TxBlobHashes);
    impl_reader_func!(fn tx_blob_gas_price() -> u64, ContractInput::TxBlobGasPrice);
}

#[cfg(test)]
mod test {
    use crate::{
        evm::{ContractInput, ExecutionContext},
        SDK,
        U256,
    };
    use alloc::vec;
    use alloy_primitives::B256;
    use fluentbase_codec::Encoder;

    #[test]
    fn test_encode_decode() {
        // encode input and put into global var
        let contract_input = ContractInput {
            contract_input: vec![0, 1, 2, 3],
            contract_bytecode: vec![4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
            block_hash: B256::from(U256::from(7)),
            ..Default::default()
        };
        let encoded_input = contract_input.encode_to_vec(0);
        SDK::with_test_input(encoded_input);
        // read input fields
        let input = ExecutionContext::contract_input();
        assert_eq!(input, contract_input.contract_input);
        let bytecode = ExecutionContext::contract_bytecode();
        assert_eq!(bytecode, contract_input.contract_bytecode);
        let block_hash = ExecutionContext::block_hash();
        assert_eq!(block_hash, contract_input.block_hash);
    }
}
