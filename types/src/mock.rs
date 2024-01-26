use crate::{Account, AccountDb, TrieDb, U256};
use alloy_primitives::{Address, Bytes, B256};
use hashbrown::HashMap;

#[derive(Default)]
pub struct InMemoryAccountDb {
    accounts: HashMap<Address, Account>,
    nodes: HashMap<Bytes, Bytes>,
    preimages: HashMap<Bytes, Bytes>,
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

    fn transfer(&mut self, _from: &Address, _to: &Address, _value: &U256) -> bool {
        todo!()
    }

    fn emit_log(&mut self, _address: &Address, _topics: &[B256], _data: Bytes) {
        todo!()
    }
}

impl TrieDb for InMemoryAccountDb {
    fn get_node(&mut self, key: &[u8]) -> Option<Bytes> {
        self.nodes.get(&Bytes::copy_from_slice(key)).cloned()
    }

    fn update_node(&mut self, key: &[u8], value: Bytes) {
        self.nodes.insert(Bytes::copy_from_slice(key), value);
    }

    fn get_preimage(&mut self, key: &[u8]) -> Option<Bytes> {
        self.preimages.get(&Bytes::copy_from_slice(key)).cloned()
    }

    fn update_preimage(&mut self, key: &[u8], value: Bytes) {
        self.preimages.insert(Bytes::copy_from_slice(key), value);
    }
}
