use crate::{
    Account,
    AccountCheckpoint,
    LowLevelSDK,
    SharedAPI,
    JZKT_ACCOUNT_BALANCE_FIELD,
    JZKT_ACCOUNT_COMPRESSION_FLAGS,
    JZKT_ACCOUNT_NONCE_FIELD,
    JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD,
    JZKT_ACCOUNT_RWASM_CODE_SIZE_FIELD,
    JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD,
    JZKT_ACCOUNT_SOURCE_CODE_SIZE_FIELD,
};
use alloc::{vec, vec::Vec};
use byteorder::{ByteOrder, LittleEndian};
use fluentbase_codec::{BufferDecoder, Encoder};
use fluentbase_codec_derive::Codec;
use fluentbase_types::{Address, Bytes, Bytes32, B256, U256};

pub trait ContextReader {
    fn block_chain_id(&self) -> u64;
    fn block_coinbase(&self) -> Address;
    fn block_timestamp(&self) -> u64;
    fn block_number(&self) -> u64;
    fn block_difficulty(&self) -> u64;
    fn block_gas_limit(&self) -> u64;
    fn block_base_fee(&self) -> U256;
    fn tx_gas_limit(&self) -> u64;
    fn tx_nonce(&self) -> u64;
    fn tx_gas_price(&self) -> U256;
    fn tx_caller(&self) -> Address;
    fn tx_access_list(&self) -> Vec<(Address, Vec<U256>)>;
    fn tx_gas_priority_fee(&self) -> Option<U256>;
    fn tx_blob_hashes(&self) -> Vec<B256>;
    fn tx_blob_hashes_size(&self) -> (u32, u32);
    fn tx_max_fee_per_blob_gas(&self) -> Option<U256>;
    fn contract_gas_limit(&self) -> u64;
    fn contract_address(&self) -> Address;
    fn contract_caller(&self) -> Address;
    fn contract_value(&self) -> U256;
    fn contract_is_static(&self) -> bool;
}

#[derive(Clone, Debug, Default, Codec)]
pub struct ContractInput {
    // block info
    pub block_chain_id: u64,
    pub block_coinbase: Address,
    pub block_timestamp: u64,
    pub block_number: u64,
    pub block_difficulty: u64,
    pub block_gas_limit: u64,
    pub block_base_fee: U256,
    // tx info
    pub tx_gas_limit: u64,
    pub tx_nonce: u64,
    pub tx_gas_price: U256,
    pub tx_gas_priority_fee: Option<U256>,
    pub tx_caller: Address,
    pub tx_access_list: Vec<(Address, Vec<U256>)>,
    pub tx_blob_hashes: Vec<B256>,
    pub tx_max_fee_per_blob_gas: Option<U256>,
    // contract info
    pub contract_gas_limit: u64,
    pub contract_address: Address,
    pub contract_caller: Address,
    pub contract_value: U256,
    pub contract_is_static: bool,
}

impl ContractInput {
    pub fn clone_from_cr<CR: ContextReader>(cr: &CR) -> ContractInput {
        ContractInput {
            block_chain_id: cr.block_chain_id(),
            block_coinbase: cr.block_coinbase(),
            block_timestamp: cr.block_timestamp(),
            block_number: cr.block_number(),
            block_difficulty: cr.block_difficulty(),
            block_gas_limit: cr.block_gas_limit(),
            block_base_fee: cr.block_base_fee(),
            tx_gas_limit: cr.tx_gas_limit(),
            tx_nonce: cr.tx_nonce(),
            tx_gas_price: cr.tx_gas_price(),
            tx_gas_priority_fee: cr.tx_gas_priority_fee(),
            tx_caller: cr.tx_caller(),
            tx_access_list: cr.tx_access_list(),
            tx_blob_hashes: cr.tx_blob_hashes(),
            tx_max_fee_per_blob_gas: cr.tx_max_fee_per_blob_gas(),
            contract_gas_limit: cr.contract_gas_limit(),
            contract_address: cr.contract_address(),
            contract_caller: cr.contract_caller(),
            contract_value: cr.contract_value(),
            contract_is_static: cr.contract_is_static(),
        }
    }
}

impl ContextReader for ContractInput {
    fn block_chain_id(&self) -> u64 {
        self.block_chain_id
    }

    fn block_coinbase(&self) -> Address {
        self.block_coinbase
    }

    fn block_timestamp(&self) -> u64 {
        self.block_timestamp
    }

    fn block_number(&self) -> u64 {
        self.block_number
    }

    fn block_difficulty(&self) -> u64 {
        self.block_difficulty
    }

    fn block_gas_limit(&self) -> u64 {
        self.block_gas_limit
    }

    fn block_base_fee(&self) -> U256 {
        self.block_base_fee
    }

    fn tx_gas_limit(&self) -> u64 {
        self.tx_gas_limit
    }

    fn tx_nonce(&self) -> u64 {
        self.tx_nonce
    }

    fn tx_gas_price(&self) -> U256 {
        self.tx_gas_price
    }

    fn tx_caller(&self) -> Address {
        self.tx_caller
    }

    fn tx_access_list(&self) -> Vec<(Address, Vec<U256>)> {
        self.tx_access_list.clone()
    }

    fn tx_gas_priority_fee(&self) -> Option<U256> {
        self.tx_gas_priority_fee
    }

    fn tx_blob_hashes(&self) -> Vec<B256> {
        self.tx_blob_hashes.clone()
    }

    fn tx_blob_hashes_size(&self) -> (u32, u32) {
        (0, self.tx_blob_hashes.len() as u32 * 32)
    }

    fn tx_max_fee_per_blob_gas(&self) -> Option<U256> {
        self.tx_max_fee_per_blob_gas.clone()
    }

    fn contract_gas_limit(&self) -> u64 {
        self.contract_gas_limit
    }

    fn contract_address(&self) -> Address {
        self.contract_address
    }

    fn contract_caller(&self) -> Address {
        self.contract_caller
    }

    fn contract_value(&self) -> U256 {
        self.contract_value
    }

    fn contract_is_static(&self) -> bool {
        self.contract_is_static
    }
}

macro_rules! impl_reader_helper {
    (@header $input_type:ty, $return_typ:ty) => {
        let mut buffer: [u8; <$input_type>::FIELD_SIZE] = [0; <$input_type>::FIELD_SIZE];
        LowLevelSDK::read_context(
            buffer.as_mut_ptr(),
            <$input_type>::FIELD_OFFSET as u32,
            buffer.len() as u32,
        );
        let mut result: $return_typ = Default::default();
        _ = <$input_type>::decode_field_header_at(&buffer, 0, &mut result);
        result
    };
    (@dynamic $input_type:ty, $return_typ:ty) => {
        let mut buffer: [u8; <$input_type>::FIELD_SIZE] = [0; <$input_type>::FIELD_SIZE];
        LowLevelSDK::read_context(
            buffer.as_mut_ptr(),
            <$input_type>::FIELD_OFFSET as u32,
            buffer.len() as u32,
        );
        let mut result: $return_typ = Default::default();
        let (offset, length) = <$input_type>::decode_field_header_at(&buffer, 0, &mut result);
        if length > 0 {
            let mut buffer2 = vec![0; offset + length];
            buffer2[0..<$input_type>::FIELD_SIZE].copy_from_slice(&buffer);
            let buffer3 = &mut buffer2.as_mut_slice()[offset..(offset + length)];
            LowLevelSDK::read_context(buffer3.as_mut_ptr(), offset as u32, buffer3.len() as u32);
            <$input_type>::decode_field_body_at(&buffer2, 0, &mut result);
        }
        result
    };
    (@size $input_type:ty, $return_typ:ty) => {
        let mut buffer: [u8; <$input_type>::FIELD_SIZE] = [0; <$input_type>::FIELD_SIZE];
        LowLevelSDK::read_context(
            buffer.as_mut_ptr(),
            <$input_type>::FIELD_OFFSET as u32,
            buffer.len() as u32,
        );
        let mut result: $return_typ = Default::default();
        let (offset, length) = <$input_type>::decode_field_header_at(&buffer, 0, &mut result);
        (offset as u32, length as u32)
    };
}

macro_rules! impl_reader_func {
    (fn $fn_name:ident() -> $return_typ:ty, $input_type:ty) => {
        paste::paste! {
            #[inline(always)]
            fn $fn_name(&self) -> $return_typ {
                impl_reader_helper!{@header <ContractInput as IContractInput>::$input_type, $return_typ}
            }
        }
    };
    (fn $fn_name:ident(result: &mut $return_typ:ty), $input_type:ty) => {
        paste::paste! {
            #[inline(always)]
            fn $fn_name(&self, result: &mut $return_typ) {
                let mut buffer: [u8; <<ContractInput as IContractInput>::$input_type>::FIELD_SIZE] = [0; <<ContractInput as IContractInput>::$input_type>::FIELD_SIZE];
                LowLevelSDK::sys_context(buffer.as_mut_ptr(), <<ContractInput as IContractInput>::$input_type>::FIELD_OFFSET as u32, buffer.len() as u32);
                _ = <<ContractInput as IContractInput>::$input_type>::decode_field_header_at(&buffer, 0, result);
            }
        }
    };
    (@dynamic fn $fn_name:ident() -> $return_typ:ty, $input_type:ty) => {
        paste::paste! {
            #[inline(always)]
            fn $fn_name(&self) -> $return_typ {
                impl_reader_helper!{@dynamic <ContractInput as IContractInput>::$input_type, $return_typ}
            }
            #[inline(always)]
            fn [<$fn_name _size>](&self) -> (u32, u32) {
                impl_reader_helper!{@size <ContractInput as IContractInput>::$input_type, $return_typ}
            }
        }
    };
}

#[derive(Default, Copy, Clone)]
pub struct ExecutionContext;

impl ExecutionContext {
    pub const DEFAULT: ExecutionContext = ExecutionContext {};
}

impl ContextReader for ExecutionContext {
    // block info
    impl_reader_func!(fn block_chain_id() -> u64, BlockChainId);
    impl_reader_func!(fn block_coinbase() -> Address, BlockCoinbase);
    impl_reader_func!(fn block_timestamp() -> u64, BlockTimestamp);
    impl_reader_func!(fn block_number() -> u64, BlockNumber);
    impl_reader_func!(fn block_difficulty() -> u64, BlockDifficulty);
    impl_reader_func!(fn block_gas_limit() -> u64, BlockGasLimit);
    impl_reader_func!(fn block_base_fee() -> U256, BlockBaseFee);
    // tx info
    impl_reader_func!(fn tx_gas_limit() -> u64, TxGasLimit);
    impl_reader_func!(fn tx_nonce() -> u64, TxNonce);
    impl_reader_func!(fn tx_gas_price() -> U256, TxGasPrice);
    impl_reader_func!(fn tx_gas_priority_fee() -> Option<U256>, TxGasPriorityFee);
    impl_reader_func!(fn tx_caller() -> Address, TxCaller);
    impl_reader_func!(fn tx_access_list() -> Vec<(Address, Vec<U256>)>, TxAccessList);
    impl_reader_func!(@dynamic fn tx_blob_hashes() -> Vec<B256>, TxBlobHashes);
    impl_reader_func!(fn tx_max_fee_per_blob_gas() -> Option<U256>, TxMaxFeePerBlobGas);
    // contract info
    impl_reader_func!(fn contract_gas_limit() -> u64, ContractGasLimit);
    impl_reader_func!(fn contract_address() -> Address, ContractAddress);
    impl_reader_func!(fn contract_caller() -> Address, ContractCaller);
    impl_reader_func!(fn contract_value() -> U256, ContractValue);
    impl_reader_func!(fn contract_is_static() -> bool, ContractIsStatic);
}

impl ExecutionContext {
    pub fn fast_return_and_exit<R: Into<Bytes>>(&self, return_data: R, exit_code: i32) {
        LowLevelSDK::write(return_data.into().as_ref());
        LowLevelSDK::exit(exit_code);
    }

    pub fn raw_input() -> Bytes {
        let input_size = LowLevelSDK::input_size();
        let mut buffer = vec![0u8; input_size as usize];
        LowLevelSDK::read(&mut buffer, 0);
        buffer.into()
    }
}
