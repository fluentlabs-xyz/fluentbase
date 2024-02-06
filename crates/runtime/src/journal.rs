use crate::TrieStorage;
use fluentbase_types::{Address, Bytes, ExitCode, B256};
use hashbrown::HashMap;
use std::mem::take;

enum JournalEvent {
    ItemChanged {
        key: [u8; 32],
        value: Vec<[u8; 32]>,
        flags: u32,
        prev_state: Option<usize>,
    },
    ItemRemoved {
        key: [u8; 32],
        prev_state: Option<usize>,
    },
}

impl JournalEvent {
    fn key(&self) -> &[u8; 32] {
        match self {
            JournalEvent::ItemChanged { key, .. } => key,
            JournalEvent::ItemRemoved { key, .. } => key,
        }
    }

    fn value(&self) -> Option<(Vec<[u8; 32]>, u32)> {
        match self {
            JournalEvent::ItemChanged { value, flags, .. } => Some((value.clone(), *flags)),
            JournalEvent::ItemRemoved { .. } => None,
        }
    }

    fn prev_state(&self) -> Option<usize> {
        match self {
            JournalEvent::ItemChanged { prev_state, .. } => *prev_state,
            JournalEvent::ItemRemoved { prev_state, .. } => *prev_state,
        }
    }
}

pub struct JournalCheckpoint(u32, u32);

impl JournalCheckpoint {
    fn state(&self) -> usize {
        self.0 as usize
    }

    fn logs(&self) -> usize {
        self.1 as usize
    }
}

pub struct JournalLog {
    address: Address,
    topics: Vec<B256>,
    data: Bytes,
}

pub struct JournaledTrie<'a, DB: TrieStorage> {
    storage: &'a mut DB,
    state: HashMap<[u8; 32], usize>,
    logs: Vec<JournalLog>,
    journal: Vec<JournalEvent>,
    root: [u8; 32],
    committed: usize,
}

impl<'a, DB: TrieStorage> JournaledTrie<'a, DB> {
    pub fn new(storage: &'a mut DB) -> Self {
        let root = storage.compute_root();
        Self {
            storage,
            state: HashMap::new(),
            logs: Vec::new(),
            journal: Vec::new(),
            root,
            committed: 0,
        }
    }

    pub fn checkpoint(&mut self) -> JournalCheckpoint {
        JournalCheckpoint(self.journal.len() as u32, 0)
    }

    pub fn get(&self, key: &[u8; 32]) -> Option<Vec<[u8; 32]>> {
        match self.state.get(key) {
            Some(index) => self.journal.get(*index).unwrap().value().map(|v| v.0),
            None => self.storage.get(key),
        }
    }

    pub fn update(&mut self, key: &[u8; 32], value: &Vec<[u8; 32]>, flags: u32) {
        let pos = self.journal.len();
        self.journal.push(JournalEvent::ItemChanged {
            key: *key,
            value: value.clone(),
            flags,
            prev_state: self.state.get(key).copied(),
        });
        self.state.insert(*key, pos);
    }

    pub fn remove(&mut self, key: &[u8; 32]) {
        let pos = self.journal.len();
        self.journal.push(JournalEvent::ItemRemoved {
            key: *key,
            prev_state: self.state.get(key).copied(),
        });
        self.state.insert(*key, pos);
    }

    pub fn compute_root(&self) -> [u8; 32] {
        self.storage.compute_root()
    }

    pub fn emit_log(&mut self, address: Address, topics: Vec<B256>, data: Bytes) {
        self.logs.push(JournalLog {
            address,
            topics,
            data,
        });
    }

    pub fn commit(&mut self) -> Result<([u8; 32], Vec<JournalLog>), ExitCode> {
        if self.committed >= self.journal.len() {
            panic!("nothing to commit")
        }
        for (key, value) in self
            .journal
            .iter()
            .skip(self.committed)
            .map(|v| (*v.key(), v.value()))
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
        self.journal.clear();
        let logs = take(&mut self.logs);
        self.committed = 0;
        self.root = self.storage.compute_root();
        Ok((self.root, logs))
    }

    pub fn rollback(&mut self, checkpoint: JournalCheckpoint) {
        if checkpoint.state() < self.committed {
            panic!("reverting already committed changes is not allowed")
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
}

#[cfg(test)]
mod tests {
    use crate::{journal::JournaledTrie, zktrie::ZkTrieStateDb, TrieStorage};
    use fluentbase_types::InMemoryAccountDb;

    macro_rules! bytes32 {
        ($val:expr) => {{
            let mut word: [u8; 32] = [0; 32];
            if $val.len() > 32 {
                word.copy_from_slice(&$val.as_bytes()[0..32]);
            } else {
                word[0..$val.len()].copy_from_slice($val.as_bytes());
            }
            word
        }};
    }

    fn calc_trie_root(values: Vec<([u8; 32], Vec<[u8; 32]>, u32)>) -> [u8; 32] {
        let mut db = InMemoryAccountDb::default();
        let mut zktrie = ZkTrieStateDb::new_empty(&mut db);
        values
            .iter()
            .for_each(|(key, value, flags)| zktrie.update(&key[..], *flags, value).unwrap());
        zktrie.compute_root()
    }

    #[test]
    fn test_commit_multiple_values() {
        let mut db = InMemoryAccountDb::default();
        let mut zktrie = ZkTrieStateDb::new_empty(&mut db);
        let mut journal = JournaledTrie::new(&mut zktrie);
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
        // add third key to the existing trie and commit
        journal.update(&bytes32!("key3"), &vec![bytes32!("val3")], 0);
        journal.commit().unwrap();
        assert_eq!(
            journal.compute_root(),
            calc_trie_root(vec![
                (bytes32!("key1"), vec![bytes32!("val1")], 0),
                (bytes32!("key2"), vec![bytes32!("val2")], 1),
                (bytes32!("key3"), vec![bytes32!("val3")], 0),
            ])
        );
    }

    #[test]
    fn test_commit_and_rollback() {
        let mut db = InMemoryAccountDb::default();
        let mut zktrie = ZkTrieStateDb::new_empty(&mut db);
        let mut journal = JournaledTrie::new(&mut zktrie);
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
        assert_eq!(journal.state.len(), 2);
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
        assert_eq!(journal.state.len(), 2);
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
        let mut db = InMemoryAccountDb::default();
        let mut zktrie = ZkTrieStateDb::new_empty(&mut db);
        let mut journal = JournaledTrie::new(&mut zktrie);
        let checkpoint = journal.checkpoint();
        journal.update(&bytes32!("key1"), &vec![bytes32!("val1")], 0);
        journal.update(&bytes32!("key2"), &vec![bytes32!("val2")], 1);
        journal.rollback(checkpoint);
        assert_eq!(journal.compute_root(), calc_trie_root(vec![]));
        assert_eq!(journal.state.len(), 0);
        let checkpoint = journal.checkpoint();
        journal.update(&bytes32!("key3"), &vec![bytes32!("val3")], 0);
        journal.update(&bytes32!("key4"), &vec![bytes32!("val4")], 1);
        journal.rollback(checkpoint);
        assert_eq!(journal.compute_root(), calc_trie_root(vec![]));
        assert_eq!(journal.state.len(), 0);
    }
}
