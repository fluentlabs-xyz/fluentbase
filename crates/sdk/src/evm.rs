use crate::{LowLevelAPI, LowLevelSDK};
use alloc::{vec, vec::Vec};
use fluentbase_codec::{define_codec_struct, BufferDecoder, Encoder};
pub use fluentbase_types::{Address, Bytes, B256, U256};

define_codec_struct! {
    pub struct JournalCheckpoint {
        state: u32,
        logs: u32,
    }
}

impl JournalCheckpoint {
    pub fn new(state: u32, logs: u32) -> Self {
        Self { state, logs }
    }
}

impl From<(u32, u32)> for JournalCheckpoint {
    fn from(value: (u32, u32)) -> Self {
        Self {
            state: value.0,
            logs: value.1,
        }
    }
}

#[cfg(feature = "runtime")]
impl From<fluentbase_runtime::JournalCheckpoint> for JournalCheckpoint {
    fn from(value: fluentbase_runtime::JournalCheckpoint) -> Self {
        Self {
            state: value.state() as u32,
            logs: value.logs() as u32,
        }
    }
}

define_codec_struct! {
    pub struct ContractInput {
        // journal
        journal_checkpoint: JournalCheckpoint,
        // env info
        env_chain_id: u64,
        // contract info
        contract_gas_limit: u64,
        contract_address: Address,
        contract_caller: Address,
        contract_bytecode: Bytes,
        contract_code_size: u32,
        contract_code_hash: B256,
        contract_input: Bytes,
        contract_input_size: u32,
        contract_value: U256,
        contract_is_static: bool,
        // block info
        block_hash: B256,
        block_coinbase: Address,
        block_timestamp: u64,
        block_number: u64,
        block_difficulty: u64,
        block_gas_limit: u64,
        block_base_fee: U256,
        // tx info
        tx_gas_price: U256,
        tx_gas_priority_fee: Option<U256>,
        tx_caller: Address,
        // tx_blob_hashes: Vec<B256>,
        // tx_blob_gas_price: u64,
    }
}

macro_rules! impl_reader_helper {
    (@header $input_type:ty, $return_typ:ty) => {
        let mut buffer: [u8; <$input_type>::FIELD_SIZE] = [0; <$input_type>::FIELD_SIZE];
        LowLevelSDK::sys_read(&mut buffer, <$input_type>::FIELD_OFFSET as u32);
        let mut result: $return_typ = Default::default();
        _ = <$input_type>::decode_field_header_at(&buffer, 0, &mut result);
        result
    };
    (@dynamic $input_type:ty, $return_typ:ty) => {
        let mut buffer: [u8; <$input_type>::FIELD_SIZE] = [0; <$input_type>::FIELD_SIZE];
        LowLevelSDK::sys_read(&mut buffer, <$input_type>::FIELD_OFFSET as u32);
        let mut result: $return_typ = Default::default();
        let (offset, length) = <$input_type>::decode_field_header_at(&buffer, 0, &mut result);
        if length > 0 {
            let mut buffer2 = vec![0; offset + length];
            buffer2[0..<$input_type>::FIELD_SIZE].copy_from_slice(&buffer);
            LowLevelSDK::sys_read(
                &mut buffer2.as_mut_slice()[offset..(offset + length)],
                offset as u32,
            );
            <$input_type>::decode_field_body_at(&buffer2, 0, &mut result);
        }
        result
    };
}
macro_rules! impl_reader_func {
    (fn $fn_name:ident() -> $return_typ:ty, $input_type:ty) => {
        paste::paste! {
            #[inline(always)]
            pub fn $fn_name() -> $return_typ {
                impl_reader_helper!{@header $input_type, $return_typ}
            }
        }
    };
    (fn $fn_name:ident(result: &mut $return_typ:ty), $input_type:ty) => {
        paste::paste! {
            #[inline(always)]
            pub fn $fn_name(result: &mut $return_typ) {
                let mut buffer: [u8; <$input_type>::FIELD_SIZE] = [0; <$input_type>::FIELD_SIZE];
                LowLevelSDK::sys_read(&mut buffer, <$input_type>::FIELD_OFFSET as u32);
                _ = <$input_type>::decode_field_header_at(&buffer, 0, result);
            }
        }
    };
    (@dynamic fn $fn_name:ident() -> $return_typ:ty, $input_type:ty) => {
        paste::paste! {
            #[inline(always)]
            pub fn $fn_name() -> $return_typ {
                impl_reader_helper!{@dynamic $input_type, $return_typ}
            }
        }
    };
}

#[derive(Default)]
pub struct ExecutionContext;

impl ExecutionContext {
    // journal
    impl_reader_func!(fn journal_checkpoint() -> JournalCheckpoint, <ContractInput as IContractInput>::JournalCheckpoint);
    // env info
    impl_reader_func!(fn env_chain_id() -> u64, <ContractInput as IContractInput>::EnvChainId);
    // contract info
    impl_reader_func!(fn contract_gas_limit() -> u64, <ContractInput as IContractInput>::ContractGasLimit);
    impl_reader_func!(fn contract_address() -> Address, <ContractInput as IContractInput>::ContractAddress);
    impl_reader_func!(fn contract_caller() -> Address, <ContractInput as IContractInput>::ContractCaller);
    impl_reader_func!(@dynamic fn contract_bytecode() -> Bytes, <ContractInput as IContractInput>::ContractBytecode);
    impl_reader_func!(fn contract_code_size() -> u32, <ContractInput as IContractInput>::ContractCodeSize);
    impl_reader_func!(fn contract_code_hash() -> B256, <ContractInput as IContractInput>::ContractCodeHash);
    impl_reader_func!(@dynamic fn contract_input() -> Bytes, <ContractInput as IContractInput>::ContractInput);
    impl_reader_func!(fn contract_input_size() -> u32, <ContractInput as IContractInput>::ContractInputSize);
    impl_reader_func!(fn contract_value() -> U256, <ContractInput as IContractInput>::ContractValue);
    impl_reader_func!(fn contract_is_static() -> bool, <ContractInput as IContractInput>::ContractIsStatic);
    // block info
    impl_reader_func!(fn block_hash() -> B256, <ContractInput as IContractInput>::BlockHash);
    impl_reader_func!(fn block_coinbase() -> Address, <ContractInput as IContractInput>::BlockCoinbase);
    impl_reader_func!(fn block_timestamp() -> u64, <ContractInput as IContractInput>::BlockTimestamp);
    impl_reader_func!(fn block_number() -> u64, <ContractInput as IContractInput>::BlockNumber);
    impl_reader_func!(fn block_difficulty() -> u64, <ContractInput as IContractInput>::BlockDifficulty);
    impl_reader_func!(fn block_gas_limit() -> u64, <ContractInput as IContractInput>::BlockGasLimit);
    impl_reader_func!(fn block_base_fee() -> U256, <ContractInput as IContractInput>::BlockBaseFee);
    // tx info
    impl_reader_func!(fn tx_gas_price() -> U256, <ContractInput as IContractInput>::TxGasPrice);
    impl_reader_func!(fn tx_gas_priority_fee() -> Option<U256>, <ContractInput as IContractInput>::TxGasPriorityFee);
    impl_reader_func!(fn tx_caller() -> Address, <ContractInput as IContractInput>::TxCaller);

    pub fn static_return_and_exit<const N: usize>(
        &self,
        return_data: &'static [u8; N],
        exit_code: i32,
    ) {
        LowLevelSDK::sys_write(return_data);
        LowLevelSDK::sys_halt(exit_code);
    }

    pub fn fast_return_and_exit<R: Into<Bytes>>(&self, return_data: R, exit_code: i32) {
        LowLevelSDK::sys_write(return_data.into().as_ref());
        LowLevelSDK::sys_halt(exit_code);
    }

    pub fn exit(&self, exit_code: i32) {
        LowLevelSDK::sys_halt(exit_code);
    }

    pub fn raw_input() -> Vec<u8> {
        let input_size = LowLevelSDK::sys_input_size();
        let mut buffer = vec![0u8; input_size as usize];
        LowLevelSDK::sys_read(&mut buffer, 0);
        buffer
    }

    pub fn full_contract_input() -> ContractInput {
        let input = Self::raw_input();
        let mut contract_input = ContractInput::default();
        let mut buffer_decoder = BufferDecoder::new(&input);
        ContractInput::decode_body(&mut buffer_decoder, 0, &mut contract_input);
        contract_input
    }
}

#[cfg(test)]
mod test {
    use crate::{
        evm::{ContractInput, ExecutionContext, U256},
        LowLevelSDK,
    };
    use fluentbase_codec::Encoder;
    use fluentbase_types::{Bytes, B256};

    #[test]
    fn test_encode_decode() {
        // encode input and put into global var
        let contract_input = ContractInput {
            contract_input: Bytes::from_static(&[0, 1, 2, 3]),
            contract_bytecode: Bytes::from_static(&[4, 5, 6, 7, 8, 9, 10, 11, 12, 13]),
            block_hash: B256::from(U256::from(7)),
            ..Default::default()
        };
        let encoded_input = contract_input.encode_to_vec(0);
        LowLevelSDK::with_test_input(encoded_input);
        // read input fields
        let input = ExecutionContext::contract_input();
        assert_eq!(input, contract_input.contract_input);
        let bytecode = ExecutionContext::contract_bytecode();
        assert_eq!(bytecode, contract_input.contract_bytecode);
        let block_hash = ExecutionContext::block_hash();
        assert_eq!(block_hash, contract_input.block_hash);
    }
}
