use crate::types::InMemoryTrieDb;
use crate::zktrie::ZkTrieStateDb;
use crate::TrieStorage;
use core::mem::take;
use fluentbase_poseidon::{hash_with_domain, Poseidon};
use fluentbase_types::{
    Address, Bytes, ExitCode, IJournaledTrie, JournalCheckpoint, JournalEvent, JournalLog, B256,
};
use halo2curves::bn256::Fr;
use hashbrown::HashMap;
use std::sync::{Arc, RwLock};

macro_rules! bytes32 {
    ($val:literal) => {
        bytes32!($val.as_bytes())
    };
    ($val:expr) => {{
        let mut word: [u8; 32] = [0; 32];
        if $val.len() > 32 {
            word.copy_from_slice(&$val[0..32]);
        } else {
            word[..$val.len()].copy_from_slice($val);
        }
        word
    }};
}

struct JournalTrieInner<DB: TrieStorage> {
    storage: DB,
    state: HashMap<[u8; 32], usize>,
    preimages: HashMap<[u8; 32], Vec<u8>>,
    logs: Vec<JournalLog>,
    journal: Vec<JournalEvent>,
    root: [u8; 32],
    committed: usize,
}

impl<DB: TrieStorage> JournalTrieInner<DB> {
    fn checkpoint(&self) -> JournalCheckpoint {
        JournalCheckpoint(self.journal.len() as u32, self.logs.len() as u32)
    }

    fn get(&self, key: &[u8; 32]) -> Option<(Vec<[u8; 32]>, u32, bool)> {
        match self.state.get(key) {
            Some(index) => self
                .journal
                .get(*index)
                .unwrap()
                .preimage()
                .map(|(values, flags)| (values, flags, false)),
            None => self
                .storage
                .get(key)
                .map(|(values, flags)| (values, flags, true)),
        }
    }

    fn update(&mut self, key: &[u8; 32], value: &Vec<[u8; 32]>, flags: u32) {
        let pos = self.journal.len();
        self.journal.push(JournalEvent::ItemChanged {
            key: *key,
            preimage: value.clone(),
            flags,
            prev_state: self.state.get(key).copied(),
        });
        self.state.insert(*key, pos);
    }

    fn remove(&mut self, key: &[u8; 32]) {
        let pos = self.journal.len();
        self.journal.push(JournalEvent::ItemRemoved {
            key: *key,
            prev_state: self.state.get(key).copied(),
        });
        self.state.insert(*key, pos);
    }

    fn compute_root(&self) -> [u8; 32] {
        self.storage.compute_root()
    }

    fn emit_log(&mut self, address: Address, topics: Vec<B256>, data: Bytes) {
        self.logs.push(JournalLog {
            address,
            topics,
            data,
        });
    }

    fn journal(&self) -> &Vec<JournalEvent> {
        return &self.journal;
    }

    fn commit(&mut self) -> Result<([u8; 32], Vec<JournalLog>), ExitCode> {
        for (key, value) in self
            .journal
            .iter()
            .skip(self.committed)
            .map(|v| (*v.key(), v.preimage()))
            .collect::<HashMap<_, _>>()
            .into_iter()
        {
            match value {
                Some((value, flags)) => {
                    self.storage.update(&key[..], flags, &value)?;
                }
                None => {
                    self.storage.remove(&key[..])?;
                }
            }
        }
        for (hash, preimage) in self.preimages.iter() {
            self.storage
                .update_preimage(hash, Bytes::from(preimage.clone()));
        }
        self.journal.clear();
        self.preimages.clear();
        self.state.clear();
        let logs = take(&mut self.logs);
        self.committed = 0;
        self.root = self.storage.compute_root();
        Ok((self.root, logs))
    }

    fn rollback(&mut self, checkpoint: JournalCheckpoint) {
        if checkpoint.state() < self.committed {
            panic!("reverting already committed changes is not allowed")
        } else if checkpoint.state() > self.journal.len() {
            panic!(
                "checkpoint overflow during rollback ({} > {})",
                checkpoint.state(),
                self.journal.len()
            )
        }
        self.journal
            .iter()
            .rev()
            .take(self.journal.len() - checkpoint.state())
            .for_each(|v| match v.prev_state() {
                Some(prev_state) => {
                    self.state.insert(*v.key(), prev_state);
                }
                None => {
                    self.state.remove(v.key());
                }
            });
        self.journal.truncate(checkpoint.state());
        self.logs.truncate(checkpoint.logs());
    }

    fn update_preimage(&mut self, key: &[u8; 32], field: u32, preimage: &[u8]) -> bool {
        // find and decode value and hash
        let value_hash = match self
            .get(key)
            .and_then(|(values, _flags, _is_cold)| values.get(field as usize).copied())
        {
            Some(value) => value,
            None => return false,
        };
        // value hash stored inside trie must be equal to the provided value hash
        // TODO(dmitry123): "we can't do this check here because hash can also be keccak256"
        // write new preimage value into database
        self.preimages.insert(value_hash, preimage.to_vec());
        true
    }

    fn preimage(&mut self, hash: &[u8; 32]) -> Vec<u8> {
        // maybe its just changed preimage and we have it in the state
        if let Some(preimage) = self.preimages.get(hash) {
            return preimage.clone();
        }
        // get preimage from database
        let preimage = self
            .storage
            .get_preimage(hash)
            .map(|v| v.to_vec())
            .unwrap_or_default();
        preimage
    }

    fn preimage_size(&mut self, hash: &[u8; 32]) -> u32 {
        if let Some(preimage) = self.preimages.get(hash) {
            return preimage.len() as u32;
        }
        self.storage.preimage_size(hash)
    }
}

pub struct JournaledTrie<DB: TrieStorage> {
    inner: Arc<RwLock<JournalTrieInner<DB>>>,
}

impl<DB: TrieStorage> Clone for JournaledTrie<DB> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl Default for JournaledTrie<ZkTrieStateDb<InMemoryTrieDb>> {
    fn default() -> Self {
        Self::new(ZkTrieStateDb::new(InMemoryTrieDb::default()))
    }
}

impl<DB: TrieStorage> JournaledTrie<DB> {
    const DOMAIN: Fr = Fr::zero();

    pub fn new(storage: DB) -> Self {
        let root = storage.compute_root();
        Self {
            inner: Arc::new(RwLock::new(JournalTrieInner {
                storage,
                state: HashMap::new(),
                preimages: HashMap::new(),
                logs: Vec::new(),
                journal: Vec::new(),
                root,
                committed: 0,
            })),
        }
    }

    pub fn message_hash(val: &[u8]) -> Fr {
        let mut hasher = Poseidon::<Fr, 3, 2>::new(8, 56);
        const CHUNK_LEN: usize = 31;
        for chunk in val.chunks(CHUNK_LEN).into_iter() {
            let mut buffer32: [u8; 32] = [0u8; 32];
            buffer32[..chunk.len()].copy_from_slice(chunk);
            let v = Fr::from_bytes(&buffer32).unwrap();
            hasher.update(&[v]);
        }
        hasher.squeeze()
    }

    pub fn compress_value(val: &[u8; 32]) -> Fr {
        let mut bytes32 = [0u8; 32];
        bytes32[0..16].copy_from_slice(&val[0..16]);
        let val1 = Fr::from_bytes(&bytes32).unwrap();
        bytes32[0..16].copy_from_slice(&val[16..]);
        let val2 = Fr::from_bytes(&bytes32).unwrap();
        hash_with_domain(&[val1, val2], &Self::DOMAIN)
    }

    pub fn storage_key(address: &Address, slot: &[u8; 32]) -> [u8; 32] {
        // storage key is `p(address, p(slot_0, slot_1, d), d)`
        let address = Fr::from_bytes(&address.into_word()).unwrap();
        let slot = Self::compress_value(slot);
        let key = hash_with_domain(&[address, slot], &Self::DOMAIN);
        key.to_bytes()
    }
}

impl<DB: TrieStorage> IJournaledTrie for JournaledTrie<DB> {
    fn checkpoint(&self) -> JournalCheckpoint {
        self.inner.read().unwrap().checkpoint()
    }

    fn get(&self, key: &[u8; 32]) -> Option<(Vec<[u8; 32]>, u32, bool)> {
        self.inner.read().unwrap().get(key)
    }

    fn update(&self, key: &[u8; 32], value: &Vec<[u8; 32]>, flags: u32) {
        self.inner.write().unwrap().update(key, value, flags)
    }

    fn remove(&self, key: &[u8; 32]) {
        self.inner.write().unwrap().remove(key)
    }

    fn compute_root(&self) -> [u8; 32] {
        self.inner.read().unwrap().compute_root()
    }

    fn emit_log(&self, address: Address, topics: Vec<B256>, data: Bytes) {
        self.inner.write().unwrap().emit_log(address, topics, data)
    }

    fn commit(&self) -> Result<([u8; 32], Vec<JournalLog>), ExitCode> {
        self.inner.write().unwrap().commit()
    }

    fn rollback(&self, checkpoint: JournalCheckpoint) {
        self.inner.write().unwrap().rollback(checkpoint)
    }

    fn update_preimage(&self, key: &[u8; 32], field: u32, preimage: &[u8]) -> bool {
        self.inner
            .write()
            .unwrap()
            .update_preimage(key, field, preimage)
    }

    fn preimage(&self, hash: &[u8; 32]) -> Vec<u8> {
        self.inner.write().unwrap().preimage(hash)
    }

    fn preimage_size(&self, hash: &[u8; 32]) -> u32 {
        self.inner.write().unwrap().preimage_size(hash)
    }

    fn journal(&self) -> Vec<JournalEvent> {
        self.inner.write().unwrap().journal.clone()
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        journal::{IJournaledTrie, JournaledTrie},
        types::InMemoryTrieDb,
        zktrie::ZkTrieStateDb,
        TrieStorage,
    };
    use fluentbase_poseidon::poseidon_hash;
    use fluentbase_types::JournalCheckpoint;

    fn calc_trie_root(values: Vec<([u8; 32], Vec<[u8; 32]>, u32)>) -> [u8; 32] {
        let db = InMemoryTrieDb::default();
        let mut zktrie = ZkTrieStateDb::new_empty(db);
        values
            .iter()
            .for_each(|(key, value, flags)| zktrie.update(&key[..], *flags, value).unwrap());
        zktrie.compute_root()
    }

    #[test]
    fn test_journal_u64() {
        let test_checkpoint = |a: u32, b: u32| {
            let jc = JournalCheckpoint(a, b);
            assert_eq!(JournalCheckpoint::from_u64(jc.to_u64()), jc);
        };
        test_checkpoint(100, 0);
        test_checkpoint(0, 100);
        test_checkpoint(0xffffffff, 0x7fffffff);
        test_checkpoint(0x7fffffff, 0xffffffff);
        test_checkpoint(0xffffffff, 0xffffffff);
        test_checkpoint(0xffffffff, 0);
        test_checkpoint(0, 0xffffffff);
        test_checkpoint(12312312, 74492);
    }

    #[test]
    fn test_commit_multiple_values() {
        let db = InMemoryTrieDb::default();
        let zktrie = ZkTrieStateDb::new_empty(db);
        let journal = JournaledTrie::new(zktrie);
        let key1 = bytes32!("key1");
        let key2 = bytes32!("key2");
        let key3 = bytes32!("key3");
        let val1 = bytes32!("val1");
        let val2 = bytes32!("val2");
        let val3 = bytes32!("val3");
        journal.update(&key1, &vec![val1.clone()], 0);
        journal.update(&key2, &vec![val2.clone()], 1);
        // just commit all changes w/o revert
        journal.commit().unwrap();
        assert_eq!(
            journal.compute_root(),
            calc_trie_root(vec![
                (key1, vec![val1.clone()], 0),
                (key2, vec![val2.clone()], 1),
            ])
        );
        // add third key to the existing trie and commit
        journal.update(&key3, &vec![val3], 0);
        journal.commit().unwrap();
        assert_eq!(
            journal.compute_root(),
            calc_trie_root(vec![
                (key1, vec![val1.clone()], 0),
                (key2, vec![val2.clone()], 1),
                (key3, vec![val3], 0),
            ])
        );
    }

    #[test]
    fn test_code_preimage_update_and_check() {
        let db = InMemoryTrieDb::default();
        let zktrie = ZkTrieStateDb::new_empty(db);
        let journal = JournaledTrie::new(zktrie);
        let address1 = bytes32!("address1");
        let _address1_hash = poseidon_hash(&address1);
        let code1 = vec![1, 2, 3, 4, 5, 6];
        let code1_hash = poseidon_hash(&code1);
        let mut account1_fields: [[u8; 32]; 4] = [[0u8; 32]; 4];
        account1_fields[2] = code1_hash;
        journal.update(&address1, &account1_fields.to_vec(), 12);
        assert!(journal.update_preimage(&address1, 2, &code1));
        assert_eq!(code1, journal.preimage(&code1_hash));
        journal.commit().unwrap();
        assert_eq!(code1, journal.preimage(&code1_hash));
    }

    #[test]
    fn test_commit_and_rollback() {
        let db = InMemoryTrieDb::default();
        let zktrie = ZkTrieStateDb::new_empty(db);
        let journal = JournaledTrie::new(zktrie);
        journal.update(&bytes32!("key1"), &vec![bytes32!("val1")], 0);
        journal.update(&bytes32!("key2"), &vec![bytes32!("val2")], 1);
        // just commit all changes w/o revert
        journal.commit().unwrap();
        assert_eq!(
            journal.compute_root(),
            calc_trie_root(vec![
                (bytes32!("key1"), vec![bytes32!("val1")], 0),
                (bytes32!("key2"), vec![bytes32!("val2")], 1),
            ])
        );
        // add third key to the existing trie and rollback
        let checkpoint = journal.checkpoint();
        journal.update(&bytes32!("key3"), &vec![bytes32!("val3")], 0);
        journal.rollback(checkpoint);
        assert_eq!(journal.inner.read().unwrap().state.len(), 0);
        assert_eq!(
            journal.compute_root(),
            calc_trie_root(vec![
                (bytes32!("key1"), vec![bytes32!("val1")], 0),
                (bytes32!("key2"), vec![bytes32!("val2")], 1),
            ])
        );
        // modify the same key and rollback
        let checkpoint = journal.checkpoint();
        journal.update(&bytes32!("key2"), &vec![bytes32!("Hello, World")], 0);
        journal.rollback(checkpoint);
        assert_eq!(journal.inner.read().unwrap().state.len(), 0);
        assert_eq!(
            journal.compute_root(),
            calc_trie_root(vec![
                (bytes32!("key1"), vec![bytes32!("val1")], 0),
                (bytes32!("key2"), vec![bytes32!("val2")], 1),
            ])
        );
    }

    #[test]
    fn test_rollback_to_empty() {
        let db = InMemoryTrieDb::default();
        let zktrie = ZkTrieStateDb::new_empty(db);
        let journal = JournaledTrie::new(zktrie);
        let checkpoint = journal.checkpoint();
        journal.update(&bytes32!("key1"), &vec![bytes32!("val1")], 0);
        journal.update(&bytes32!("key2"), &vec![bytes32!("val2")], 1);
        journal.rollback(checkpoint);
        assert_eq!(journal.compute_root(), calc_trie_root(vec![]));
        assert_eq!(journal.inner.read().unwrap().state.len(), 0);
        let checkpoint = journal.checkpoint();
        journal.update(&bytes32!("key3"), &vec![bytes32!("val3")], 0);
        journal.update(&bytes32!("key4"), &vec![bytes32!("val4")], 1);
        journal.rollback(checkpoint);
        assert_eq!(journal.compute_root(), calc_trie_root(vec![]));
        assert_eq!(journal.inner.read().unwrap().state.len(), 0);
    }
}
