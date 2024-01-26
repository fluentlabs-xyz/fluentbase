use crate::{LowLevelAPI, LowLevelSDK};
use alloc::{vec, vec::Vec};
use fluentbase_codec::{define_codec_struct, BufferDecoder, EmptyArray, Encoder};
pub use fluentbase_types::{Address, Bytes, B256, U256};

define_codec_struct! {
    pub struct ContractInput {
        // env info
        env_chain_id: u64,
        // contract info
        contract_address: Address,
        contract_caller: Address,
        contract_bytecode: Bytes,
        contract_code_size: u32,
        contract_code_hash: B256,
        contract_input: Bytes,
        contract_input_size: u32,
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
        tx_gas_price: U256,
        tx_gas_priority_fee: Option<U256>,
        tx_caller: Address,
        // tx_blob_hashes: Vec<B256>,
        // tx_blob_gas_price: u64,
    }
}
define_codec_struct! {
    pub struct ContractLog {
        address: Address,
        topic0: Option<[B256; 1]>,
        topic1: Option<[B256; 2]>,
        topic2: Option<[B256; 3]>,
        topic3: Option<[B256; 4]>,
        topic4: Option<[B256; 5]>,
        data: Bytes,
    }
}
define_codec_struct! {
    pub struct ContractOutput {
        return_data: Bytes,
        logs: Option<Vec<ContractLog>>,
    }
}
define_codec_struct! {
    pub struct ContractOutputNoLogs {
        return_data: Bytes,
        logs: EmptyArray,
    }
}

macro_rules! impl_reader_helper {
    ($input_type:ty, $return_typ:ty) => {{
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
    }};
}
macro_rules! impl_reader_func {
    (fn $fn_name:ident() -> $return_typ:ty, $input_type:ty) => {
        paste::paste! {
            pub fn $fn_name() -> $return_typ {
                impl_reader_helper!($input_type, $return_typ)
            }
            // pub fn [<get_ $fn_name>](&mut self) -> &$return_typ {
                // if self.cached_state.$fn_name.is_none() {
                //     self.cached_state.$fn_name = Some(impl_reader_helper!($input_type, $return_typ));
                // }
                // self.cached_state.$fn_name.as_ref().unwrap()
            // }
        }
    };
}

macro_rules! impl_emit_log {
    ($fn_name:ident, $log_field:ident, $num_topics:expr) => {
        pub fn $fn_name(&mut self, topics: [B256; $num_topics], data: Bytes) {
            let address = Self::contract_address();
            let output = output_mut_or_default!(self);
            output
                .logs
                .get_or_insert(Default::default())
                .push(ContractLog {
                    address,
                    $log_field: Some(topics),
                    data,
                    ..Default::default()
                });
        }
    };
}

// #[derive(Default)]
// struct CachedState {
//     env_chain_id: Option<u64>,
//     contract_address: Option<Address>,
//     contract_caller: Option<Address>,
//     contract_bytecode: Option<Bytes>,
//     contract_code_size: Option<u32>,
//     contract_code_hash: Option<B256>,
//     contract_input: Option<Bytes>,
//     contract_value: Option<U256>,
//     block_hash: Option<B256>,
//     block_coinbase: Option<Address>,
//     block_timestamp: Option<u64>,
//     block_number: Option<u64>,
//     block_difficulty: Option<u64>,
//     block_gas_limit: Option<u64>,
//     block_base_fee: Option<U256>,
//     tx_gas_price: Option<U256>,
//     tx_gas_priority_fee: Option<Option<U256>>,
//     tx_caller: Option<Address>,
//     tx_blob_hashes: Option<Vec<B256>>,
//     tx_blob_gas_price: Option<u64>,
// }

#[derive(Default)]
pub struct ExecutionContext {
    output: Option<ContractOutput>,
    // cached_state: CachedState,
}

macro_rules! output_mut_or_default {
    ($self:ident) => {{
        if $self.output.is_none() {
            $self.output = Some(Default::default());
        }
        $self.output.as_mut().unwrap()
    }};
}

impl ExecutionContext {
    // env info
    impl_reader_func!(fn env_chain_id() -> u64, <ContractInput as IContractInput>::EnvChainId);
    // contract info
    impl_reader_func!(fn contract_address() -> Address, <ContractInput as IContractInput>::ContractAddress);
    impl_reader_func!(fn contract_caller() -> Address, <ContractInput as IContractInput>::ContractCaller);
    impl_reader_func!(fn contract_bytecode() -> Bytes, <ContractInput as IContractInput>::ContractBytecode);
    impl_reader_func!(fn contract_code_size() -> u32, <ContractInput as IContractInput>::ContractCodeSize);
    impl_reader_func!(fn contract_code_hash() -> B256, <ContractInput as IContractInput>::ContractCodeHash);
    impl_reader_func!(fn contract_input() -> Bytes, <ContractInput as IContractInput>::ContractInput);
    impl_reader_func!(fn contract_input_size() -> u32, <ContractInput as IContractInput>::ContractInputSize);
    impl_reader_func!(fn contract_value() -> U256, <ContractInput as IContractInput>::ContractValue);
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
    // impl_reader_func!(fn tx_blob_hashes() -> Vec<B256>, ContractInput::TxBlobHashes);
    // impl_reader_func!(fn tx_blob_gas_price() -> u64, ContractInput::TxBlobGasPrice);

    impl_emit_log!(emit_log0, topic0, 1);
    impl_emit_log!(emit_log1, topic1, 2);
    impl_emit_log!(emit_log2, topic2, 3);
    impl_emit_log!(emit_log3, topic3, 4);
    impl_emit_log!(emit_log4, topic4, 5);

    pub fn emit_return(&mut self, return_data: &[u8]) {
        let output = output_mut_or_default!(self);
        output.return_data = Bytes::copy_from_slice(return_data);
    }

    pub fn static_return_and_exit<const N: usize>(
        &self,
        return_data: &'static [u8; N],
        exit_code: i32,
    ) where
        [u8; N + ContractOutput::HEADER_SIZE]:,
    {
        let contract_output = ContractOutputNoLogs {
            return_data: Bytes::from_static(return_data),
            logs: Default::default(),
        };
        let (buffer, length) =
            contract_output.encode_to_fixed::<{ N + ContractOutput::HEADER_SIZE }>(0);
        LowLevelSDK::sys_write(&buffer[..length]);
        LowLevelSDK::sys_halt(exit_code);
    }

    pub fn fast_return_and_exit<R: Into<Bytes>>(&self, return_data: R, exit_code: i32) {
        let contract_output = ContractOutput {
            return_data: return_data.into(),
            logs: None,
        };
        LowLevelSDK::sys_write(contract_output.encode_to_vec(0).as_slice());
        // LowLevelSDK::sys_write(return_data);
        LowLevelSDK::sys_halt(exit_code);
    }

    pub fn exit(&self, exit_code: i32) {
        if let Some(output) = self.output.as_ref() {
            LowLevelSDK::sys_write(output.encode_to_vec(0).as_slice());
        }
        LowLevelSDK::sys_halt(exit_code);
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
