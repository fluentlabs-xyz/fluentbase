use crate::{Account, TrieDb};
use alloy_primitives::{Address, Bytes};
use hashbrown::HashMap;

#[derive(Default, Clone)]
pub struct InMemoryTrieDb {
    accounts: HashMap<Address, Account>,
    nodes: HashMap<Bytes, Bytes>,
    preimages: HashMap<Bytes, Bytes>,
}

impl TrieDb for InMemoryTrieDb {
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
