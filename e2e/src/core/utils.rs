use fluentbase_codec::Encoder;
use fluentbase_core::{Account, JZKT_ACCOUNT_COMPRESSION_FLAGS};
use fluentbase_runtime::{DefaultEmptyRuntimeDatabase, ExecutionResult, Runtime, RuntimeContext};
use fluentbase_sdk::{evm::ContractInput, LowLevelSDK};
use fluentbase_types::{Address, Bytes, IJournaledTrie, STATE_DEPLOY, STATE_MAIN, U256};
use hashbrown::HashMap;
use paste::paste;
use rwasm::core::ImportLinker;

#[derive(Default)]
pub(crate) struct TestingContext<const IS_RUNTIME: bool> {
    accounts: HashMap<Address, Account>,
    pub contract_input_wrapper: ContractInputWrapper,
}

#[allow(dead_code)]
impl<const IS_RUNTIME: bool> TestingContext<IS_RUNTIME> {
    pub fn new() -> Self {
        Self {
            accounts: Default::default(),
            contract_input_wrapper: ContractInputWrapper::default(),
        }
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

    pub fn run_rwasm_with_input(
        &self,
        runtime_ctx: RuntimeContext<DefaultEmptyRuntimeDatabase>,
        import_linker: ImportLinker,
        is_deploy: bool,
        gas_limit: u32,
    ) -> ExecutionResult {
        let runtime_ctx = runtime_ctx
            .with_state(if is_deploy { STATE_DEPLOY } else { STATE_MAIN })
            .with_fuel_limit(gas_limit as u64)
            .with_catch_trap(true);
        let mut runtime =
            Runtime::<DefaultEmptyRuntimeDatabase>::new(runtime_ctx, import_linker).unwrap();
        runtime.data_mut().clean_output();
        runtime.call().unwrap()
    }
}

impl TestingContext<true> {
    pub fn apply_ctx(
        &mut self,
        runtime_ctx: &mut RuntimeContext<DefaultEmptyRuntimeDatabase>,
    ) -> &mut Self {
        let contract_input_vec = self.contract_input_wrapper.0.encode_to_vec(0);
        let jzkt = runtime_ctx.jzkt();
        for (address, account) in &self.accounts {
            jzkt.update(
                &address.into_word(),
                &account.get_fields().to_vec(),
                JZKT_ACCOUNT_COMPRESSION_FLAGS,
            )
        }
        runtime_ctx.change_input(contract_input_vec);
        self
    }
}

impl TestingContext<false> {
    pub fn apply_ctx(&mut self) -> &mut Self {
        let contract_input_vec = self.contract_input_wrapper.0.encode_to_vec(0);
        for (_address, account) in &self.accounts {
            account.write_to_jzkt();
        }
        LowLevelSDK::with_test_input(contract_input_vec);
        self
    }
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
    impl_once_setter!(contract_address, Address);
    impl_once_setter!(contract_caller, Address);
    impl_once_setter!(contract_value, U256);
    impl_once_setter!(contract_is_static, bool);
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
