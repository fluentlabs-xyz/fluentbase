use alloc::{rc::Rc, vec};
use core::cell::RefCell;
use fluentbase_codec::Encoder;
use fluentbase_core::{account::Account, account_types::JZKT_COMPRESSION_FLAGS};
use fluentbase_runtime::{
    types::InMemoryTrieDb,
    zktrie::ZkTrieStateDb,
    ExecutionResult,
    JournaledTrie,
    Runtime,
    RuntimeContext,
};
use fluentbase_sdk::{evm::ContractInput, LowLevelSDK};
use fluentbase_types::{Address, Bytes, B256, STATE_DEPLOY, STATE_MAIN, U256};
use hashbrown::HashMap;
use keccak_hash::keccak;
use paste::paste;
use rwasm::core::ImportLinker;
use std::marker::PhantomData;

#[derive(Default)]
pub(crate) struct TestingContext<T, const IS_RUNTIME: bool> {
    accounts: HashMap<Address, Account>,
    pub contract_input_wrapper: ContractInputWrapper,
    _type_ghost: PhantomData<T>,
}

#[allow(dead_code)]
impl<T, const IS_RUNTIME: bool> TestingContext<T, IS_RUNTIME> {
    pub fn new(init_jzkt: bool, runtime_ctx: Option<&mut RuntimeContext<'_, T>>) -> Self {
        let mut instance = Self {
            accounts: Default::default(),
            contract_input_wrapper: ContractInputWrapper::default(),
            _type_ghost: Default::default(),
        };
        if init_jzkt {
            instance.init_jzkt(runtime_ctx);
        }
        instance
    }

    pub fn reset_contract_input_wrapper(&mut self) -> &mut Self {
        self.contract_input_wrapper = Default::default();
        self
    }

    pub fn try_add_account(&mut self, account: &Account) -> &mut Self {
        self.accounts
            .try_insert(account.address, (*account).clone())
            .unwrap();
        self
    }

    pub fn get_account_mut(&mut self, address: Address) -> &mut Account {
        if !self.accounts.contains_key(&address) {
            self.accounts.insert(address, Account::default());
        }
        self.accounts.get_mut(&address).unwrap()
    }

    pub fn init_jzkt(
        &mut self,
        runtime_ctx: Option<&mut RuntimeContext<'_, T>>,
    ) -> Rc<RefCell<JournaledTrie<ZkTrieStateDb<InMemoryTrieDb>>>> {
        let db = InMemoryTrieDb::default();
        let storage = ZkTrieStateDb::new_empty(db);
        let journal = JournaledTrie::new(storage);
        let journal_ref = Rc::new(RefCell::new(journal));
        if IS_RUNTIME {
            let runtime_ctx = runtime_ctx.unwrap();
            runtime_ctx.with_jzkt(journal_ref.clone());
        } else {
            LowLevelSDK::with_jzkt(journal_ref.clone());
        }

        journal_ref
    }

    pub fn apply_ctx(&mut self, runtime_ctx: Option<&mut RuntimeContext<'_, T>>) -> &mut Self {
        let contract_input_vec = self.contract_input_wrapper.0.encode_to_vec(0);
        if IS_RUNTIME {
            let runtime_ctx = runtime_ctx.unwrap();
            let jzkt = runtime_ctx.jzkt().unwrap();
            for (address, account) in &self.accounts {
                jzkt.borrow_mut().update(
                    &address.into_word(),
                    &account.get_fields().to_vec(),
                    JZKT_COMPRESSION_FLAGS,
                )
            }
            runtime_ctx.with_input(contract_input_vec);
        } else {
            for (_address, account) in &self.accounts {
                account.write_to_jzkt();
            }
            if runtime_ctx.is_some() {
                let runtime_ctx = runtime_ctx.unwrap();
                LowLevelSDK::with_jzkt(runtime_ctx.jzkt().unwrap());
            }
            LowLevelSDK::with_test_input(contract_input_vec);
        }

        self
    }

    pub fn run_rwasm_with_input<'t>(
        &self,
        mut runtime_ctx: RuntimeContext<'t, T>,
        import_linker: ImportLinker,
        is_deploy: bool,
        gas_limit: u32,
    ) -> ExecutionResult<'t, T> {
        runtime_ctx
            .with_state(if is_deploy { STATE_DEPLOY } else { STATE_MAIN })
            .with_fuel_limit(gas_limit)
            .with_catch_trap(true);
        let mut runtime = Runtime::new(runtime_ctx, import_linker).unwrap();
        runtime.data_mut().clean_output();
        runtime.call().unwrap()
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
    ($field_name: ident, Option<$field_type: tt>) => {
        paste! {
            pub fn [<set_ $field_name>](&mut self, v: Option<$field_type>) -> &mut Self {
                if self.0.$field_name != Option::default() {
                    panic!("updating '{}' field is not allowed when it contains non-default value. use reset before updating", stringify!($field_name));
                }
                self.0.$field_name = v;

                self
            }
            pub fn [<reset_ $field_name>](&mut self) -> &mut Self {
                self.0.$field_name = Option::default();

                self
            }
        }
    };
    ($field_name: ident, $field_type: tt) => {
        paste! {
            pub fn [<set_ $field_name>](&mut self, v: $field_type) -> &mut Self {
                if self.0.$field_name != $field_type::default() {
                    panic!("updating '{}' field is not allowed when it contains non-default value. use reset before updating", stringify!($field_name));
                }
                self.0.$field_name = v;

                self
            }
            pub fn [<reset_ $field_name>](&mut self) -> &mut Self {
                self.0.$field_name = $field_type::default();

                self
            }
        }
    };
}

#[derive(Default)]
pub(crate) struct ContractInputWrapper(ContractInput);

#[allow(dead_code)]
impl ContractInputWrapper {
    impl_once_setter!(journal_checkpoint, u64);
    impl_once_setter!(env_chain_id, u64);
    impl_once_setter!(contract_gas_limit, u64);
    impl_once_setter!(contract_input, Bytes);
    impl_once_setter!(contract_input_size, u32);
    impl_once_setter!(contract_address, Address);
    impl_once_setter!(contract_caller, Address);
    impl_once_setter!(contract_value, U256);
    impl_once_setter!(contract_is_static, bool);
    impl_once_setter!(block_hash, B256);
    impl_once_setter!(block_coinbase, Address);
    impl_once_setter!(block_timestamp, u64);
    impl_once_setter!(block_number, u64);
    impl_once_setter!(block_difficulty, u64);
    impl_once_setter!(block_gas_limit, u64);
    impl_once_setter!(block_base_fee, U256);
    impl_once_setter!(tx_gas_price, U256);
    impl_once_setter!(tx_gas_priority_fee, Option<U256>);
    impl_once_setter!(tx_caller, Address);
}
