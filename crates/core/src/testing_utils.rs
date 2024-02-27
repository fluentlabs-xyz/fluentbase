use crate::account::Account;
use alloc::{rc::Rc, vec};
use core::cell::RefCell;
use fluentbase_codec::Encoder;
use fluentbase_runtime::{types::B256, zktrie::ZkTrieStateDb, JournaledTrie};
use fluentbase_sdk::{evm::ContractInput, LowLevelSDK};
use fluentbase_types::{Address, Bytes, InMemoryAccountDb, U256};
use hashbrown::HashMap;
use keccak_hash::keccak;

#[derive(Default)]
pub(crate) struct TestingContext {
    accounts: HashMap<Address, Account>,
    pub contract_input_wrapper: ContractInputWrapper,
}

impl TestingContext {
    pub fn new() -> Self {
        Self {
            accounts: Default::default(),
            contract_input_wrapper: ContractInputWrapper::default(),
        }
    }

    pub fn reset_contract_input_wrapper(&mut self) -> &mut Self {
        self.contract_input_wrapper = Default::default();
        self
    }

    pub fn try_add_account(&mut self, address: Address, account: Account) -> &mut Self {
        self.accounts.try_insert(address, account).unwrap();
        self
    }

    pub fn get_account_mut(&mut self, address: Address) -> &mut Account {
        if !self.accounts.contains_key(&address) {
            self.accounts.insert(address, Account::default());
        }
        self.accounts.get_mut(&address).unwrap()
    }

    pub fn apply_ctx(&mut self) -> &mut Self {
        let db = InMemoryAccountDb::default();
        let storage = ZkTrieStateDb::new_empty(db);
        let journal = JournaledTrie::new(storage);
        LowLevelSDK::with_jzkt(Rc::new(RefCell::new(journal)));

        for (_address, account) in &self.accounts {
            account.write_to_jzkt();
        }

        let contract_input_vec = self.contract_input_wrapper.0.encode_to_vec(0);
        LowLevelSDK::with_test_input(contract_input_vec);

        self
    }
}

pub(crate) fn generate_address_original_impl(address: &Address, nonce: u64) -> Address {
    use alloy_rlp::Encodable;
    let mut out = vec![];
    alloy_rlp::Header {
        list: true,
        payload_length: address.length() + nonce.length(),
    }
    .encode(&mut out);
    Encodable::encode(&address, &mut out);
    Encodable::encode(&nonce, &mut out);
    out.resize(32, 0);
    Address::from_word(keccak(out).0.into())
}

macro_rules! impl_once_setter {
    ($setter_name: ident, $field_name: ident, Option<$field_type: tt>) => {
        pub fn $setter_name(&mut self, v: Option<$field_type>) -> &mut Self {
            if self.0.$field_name != Option::default() {
                panic!("cannot set field twice")
            }
            self.0.$field_name = v;

            self
        }
    };
    ($setter_name: ident, $field_name: ident, $field_type: tt) => {
        pub fn $setter_name(&mut self, v: $field_type) -> &mut Self {
            if self.0.$field_name != $field_type::default() {
                panic!("cannot set field twice")
            }
            self.0.$field_name = v;

            self
        }
    };
}

#[derive(Default)]
pub(crate) struct ContractInputWrapper(ContractInput);

impl ContractInputWrapper {
    impl_once_setter!(set_contract_input, contract_input, Bytes);
    impl_once_setter!(set_contract_input_size, contract_input_size, u32);
    impl_once_setter!(set_env_chain_id, env_chain_id, u64);

    impl_once_setter!(set_contract_address, contract_address, Address);
    impl_once_setter!(set_contract_caller, contract_caller, Address);
    impl_once_setter!(set_contract_bytecode, contract_bytecode, Bytes);
    impl_once_setter!(set_contract_code_size, contract_code_size, u32);
    impl_once_setter!(set_contract_code_hash, contract_code_hash, B256);
    impl_once_setter!(set_contract_value, contract_value, U256);
    impl_once_setter!(set_contract_is_static, contract_is_static, bool);
    impl_once_setter!(set_block_hash, block_hash, B256);
    impl_once_setter!(set_block_coinbase, block_coinbase, Address);
    impl_once_setter!(set_block_timestamp, block_timestamp, u64);
    impl_once_setter!(set_block_number, block_number, u64);
    impl_once_setter!(set_block_difficulty, block_difficulty, u64);
    impl_once_setter!(set_block_gas_limit, block_gas_limit, u64);
    impl_once_setter!(set_block_base_fee, block_base_fee, U256);
    impl_once_setter!(set_tx_gas_price, tx_gas_price, U256);
    impl_once_setter!(set_tx_gas_priority_fee, tx_gas_priority_fee, Option<U256>);
    impl_once_setter!(set_tx_caller, tx_caller, Address);
}
