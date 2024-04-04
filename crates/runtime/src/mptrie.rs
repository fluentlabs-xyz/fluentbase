use std::ops::Deref;
use std::{cell::RefCell, sync::Arc};

use eth_trie::{EthTrie, Trie, DB};
use hex_literal::hex;
use keccak_hash::H256;

use fluentbase_types::{Bytes, ExitCode};
use fluentbase_zktrie::{Database, PreimageDatabase};

use crate::{storage::TrieStorage, types::TrieDb};

pub const EMPTY_ROOT_HASH: [u8; 32] =
    hex!("56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421");

pub struct MPTrieStateDb<DB: eth_trie::DB + TrieDb> {
    storage: Arc<DB>,
    trie: Option<RefCell<EthTrie<DB>>>,
}

impl<DB: eth_trie::DB + TrieDb> MPTrieStateDb<DB> {
    pub fn new(storage: Arc<DB>) -> Self {
        Self {
            storage,
            trie: None,
        }
    }

    pub fn new_empty(storage: Arc<DB>) -> Self {
        Self::new_opened(storage, &EMPTY_ROOT_HASH)
    }

    pub fn new_opened(storage: Arc<DB>, root32: &[u8]) -> Self {
        let mut storage = Self::new(storage);
        storage.open(root32);
        storage
    }
}

impl<DB: eth_trie::DB + TrieDb> TrieStorage for MPTrieStateDb<DB> {
    fn open(&mut self, root32: &[u8]) -> bool {
        if self.trie.as_ref().is_some() {
            return false;
        }
        let mut trie = EthTrie::new(self.storage.clone());
        if root32 != EMPTY_ROOT_HASH {
            trie = trie.at_root(H256::from_slice(root32))
        };
        self.trie = Some(RefCell::new(trie));
        true
    }

    fn compute_root(&self) -> [u8; 32] {
        let mut trie = self.trie.as_ref().unwrap().borrow_mut();
        trie.root_hash().map_or(EMPTY_ROOT_HASH, |v| v.0)
    }

    fn get(&self, key: &[u8]) -> Option<(Vec<[u8; 32]>, u32)> {
        let mut trie = self.trie.as_ref().unwrap().borrow_mut();
        if let Ok(val) = trie.get(key) {
            match val {
                Some(data) => {
                    let result = data
                        .to_vec()
                        .chunks(32)
                        .map(|val| {
                            let mut bytes = [0u8; 32];
                            bytes.copy_from_slice(val);
                            bytes
                        })
                        .collect::<Vec<_>>();
                    Some((result, 0))
                }
                _ => None,
            }
        } else {
            None
        }
    }

    fn update(
        &mut self,
        key: &[u8],
        _value_flags: u32,
        value: &Vec<[u8; 32]>,
    ) -> Result<(), ExitCode> {
        let mut trie = self.trie.as_ref().unwrap().borrow_mut();
        let mut value_res: Vec<u8> = Vec::with_capacity(value.len() * 32);
        value.iter().for_each(|v| value_res.extend_from_slice(v));
        trie.insert(key, &value_res).unwrap();
        Ok(())
    }

    fn remove(&mut self, key: &[u8]) -> Result<(), ExitCode> {
        let mut trie = self.trie.as_ref().unwrap().borrow_mut();
        let r = trie.remove(key);
        match r {
            Ok(_) => Ok(()),
            Err(_) => Err(ExitCode::PersistentStorageError),
        }
    }

    fn proof(&self, key: &[u8; 32]) -> Option<Vec<Vec<u8>>> {
        let mut trie = self.trie.as_ref().unwrap().borrow_mut();
        let p = trie.get_proof(key);
        p.map_or(None, |v| Some(v))
    }

    fn get_preimage(&mut self, key: &[u8]) -> Option<Bytes> {
        let r = self.storage.get(key).unwrap_or_default();
        r.map_or(None, |v| Some(Bytes::copy_from_slice(&v)))
    }

    fn update_preimage(&mut self, key: &[u8], value: Bytes) {
        let _ = self.storage.insert(key, value.to_vec());
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use eth_trie::MemoryDB;

    use crate::mptrie::MPTrieStateDb;
    use crate::TrieStorage;

    macro_rules! bytes32 {
        ($val:expr) => {{
            let mut word: [u8; 32] = [0; 32];
            if $val.len() > 32 {
                word.copy_from_slice(&$val.as_bytes()[0..32]);
            } else {
                word[0..$val.len()].copy_from_slice($val.as_bytes());
            }
            Box::leak(Box::new(word))
        }};
    }

    #[test]
    fn test_simple() {
        let mut state_db1 = MPTrieStateDb::new_empty(Arc::new(MemoryDB::new(true)));
        state_db1
            .update(
                bytes32!("key1"),
                0,
                &vec![*bytes32!("value1"), *bytes32!("value2")],
            )
            .unwrap();
        let root = state_db1.compute_root();
        println!("root: {:?}", hex::encode(root));
        let state_db2 = MPTrieStateDb::new_opened(state_db1.storage, &root);
        let (data, _flags) = state_db2.get(bytes32!("key1")).unwrap();
        assert_eq!(data[0], *bytes32!("value1"));
        assert_eq!(data[1], *bytes32!("value2"));
    }
}
