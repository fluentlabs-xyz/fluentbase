use crate::evm::{EvmImpl, EVM};
use fluentbase_sdk::{AccountManager, Address, Bytes, ContextReader, U256};

impl<'a, CR: ContextReader, AM: AccountManager> EVM for EvmImpl<'a, CR, AM> {
    fn address(&self) -> Address {
        self.cr.contract_address()
    }

    fn balance(&self, address: Address) -> U256 {
        let (account, _) = self.am.account(address);
        account.balance
    }

    fn call(&self, _callee: Address, _value: U256, _input: Bytes, _gas: u64) {
        todo!()
    }

    fn sload(&self, index: U256) -> U256 {
        let contract_address = self.cr.contract_address();
        let (value, _) = self.am.storage(contract_address, index, false);
        value
    }

    fn sstore(&self, index: U256, value: U256) {
        let contract_address = self.cr.contract_address();
        _ = self.am.write_storage(contract_address, index, value);
    }
}
