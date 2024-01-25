use crate::{Account, AccountDb, U256};
use alloy_primitives::Address;
use std::collections::HashMap;

#[derive(Default)]
pub struct InMemoryAccountDb {
    accounts: HashMap<Address, Account>,
    nodes: HashMap<Vec<u8>, Vec<u8>>,
    preimages: HashMap<Vec<u8>, Vec<u8>>,
}

impl AccountDb for InMemoryAccountDb {
    fn get_account(&mut self, address: &Address) -> Option<Account> {
        self.accounts.get(address).cloned()
    }

    fn update_account(&mut self, address: &Address, account: &Account) {
        self.accounts.insert(address.clone(), account.clone());
    }

    fn get_storage(&mut self, _address: &Address, _index: &U256) -> Option<U256> {
        todo!()
    }

    fn update_storage(&mut self, _address: &Address, _index: &U256, _value: &U256) {
        todo!()
    }

    fn get_node(&mut self, key: &[u8]) -> Option<Vec<u8>> {
        self.nodes.get(&key.to_vec()).cloned()
    }

    fn update_node(&mut self, key: &[u8], value: &Vec<u8>) {
        self.nodes.insert(key.to_vec(), value.to_vec());
    }

    fn get_preimage(&mut self, key: &[u8]) -> Option<Vec<u8>> {
        self.preimages.get(&key.to_vec()).cloned()
    }

    fn update_preimage(&mut self, key: &[u8], value: &Vec<u8>) {
        self.preimages.insert(key.to_vec(), value.to_vec());
    }
}
