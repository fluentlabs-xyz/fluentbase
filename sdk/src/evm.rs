use crate::{SysPlatformSDK, SDK};
use alloc::{vec, vec::Vec};
pub use alloy_primitives::{Address, Bytes, B256, U256};
use fluentbase_codec::define_codec_struct;

define_codec_struct! {
    pub struct ContractInput {
        input: Vec<u8>,
        bytecode: Vec<u8>,
        hash: B256,
        address: Address,
        caller: Address,
        value: U256,
        block_hash: B256,
        balance: U256,
    }
}

macro_rules! impl_reader_helper {
    ($fn_name:ident, $input_type:ty, $return_typ:ty) => {
        pub fn $fn_name() -> $return_typ {
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
        }
    };
}

impl_reader_helper!(contract_read_input, ContractInput::Input, Vec<u8>);
impl_reader_helper!(contract_read_bytecode, ContractInput::Bytecode, Vec<u8>);
impl_reader_helper!(contract_read_hash, ContractInput::Hash, B256);
impl_reader_helper!(contract_read_address, ContractInput::Address, Address);
impl_reader_helper!(contract_read_caller, ContractInput::Caller, Address);
impl_reader_helper!(contract_read_value, ContractInput::Value, U256);
impl_reader_helper!(contract_read_block_hash, ContractInput::BlockHash, B256);
impl_reader_helper!(contract_read_balance, ContractInput::Balance, U256);

#[cfg(test)]
mod test {
    use crate::{
        evm::{
            contract_read_block_hash,
            contract_read_bytecode,
            contract_read_input,
            ContractInput,
        },
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
            input: vec![0, 1, 2, 3],
            bytecode: vec![4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
            hash: Default::default(),
            address: Default::default(),
            caller: Default::default(),
            value: Default::default(),
            block_hash: B256::from(U256::from(7)),
            balance: Default::default(),
        };
        let encoded_input = contract_input.encode_to_vec(0);
        // for chunk in encoded_input.chunks(32) {
        //     println!("{}", hex::encode(chunk));
        // }
        SDK::with_test_input(encoded_input);
        // read input fields
        let input = contract_read_input();
        assert_eq!(input, contract_input.input);
        let bytecode = contract_read_bytecode();
        assert_eq!(bytecode, contract_input.bytecode);
        let block_hash = contract_read_block_hash();
        assert_eq!(block_hash, contract_input.block_hash);
    }
}
