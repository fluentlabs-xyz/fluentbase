use crate::storage::TrieStorage;
use fluentbase_types::{Bytes, ExitCode, TrieDb, POSEIDON_EMPTY};
use fluentbase_zktrie::{
    Byte32,
    Database,
    Error,
    Hash,
    Node,
    PoseidonHash,
    PreimageDatabase,
    TrieData,
    ZkTrie,
};
use halo2curves::bn256::Fr;
use std::{cell::RefCell, rc::Rc, sync::Arc};

#[derive(Clone)]
struct NodeDb<DB>(Rc<RefCell<DB>>);

const STORAGE_PREFIX_NODE: u8 = 0x01;
const STORAGE_PREFIX_PREIMAGE: u8 = 0x02;
const STORAGE_PREFIX_CODE: u8 = 0x02;

macro_rules! storage_key {
    ($prefix:ident, $key:expr) => {{
        assert_eq!($key.len(), 32);
        let mut storage_key = [0u8; 33];
        storage_key[0] = $prefix;
        storage_key[1..].copy_from_slice($key);
        storage_key
    }};
}

impl<DB: TrieDb> Database for NodeDb<DB> {
    type Node = Node<PoseidonHash>;

    fn get_node(&self, key: &Hash) -> Result<Option<Arc<Self::Node>>, Error> {
        match self.0.borrow_mut().get_node(key.raw_bytes()) {
            Some(value) => Ok(Some(Arc::new(Node::from_bytes(&value)?))),
            None => Ok(None),
        }
    }

    fn update_node(&mut self, node: Self::Node) -> Result<Arc<Self::Node>, Error> {
        self.0.borrow_mut().update_node(
            node.hash().raw_bytes(),
            Bytes::copy_from_slice(&node.canonical_value()),
        );
        Ok(Arc::new(node))
    }
}

impl<'a, DB: TrieDb> PreimageDatabase for NodeDb<DB> {
    fn update_preimage(&mut self, preimage: &[u8], hash_field: &Fr) {
        self.0
            .borrow_mut()
            .update_preimage(&hash_field.to_bytes(), Bytes::copy_from_slice(preimage));
    }

    fn preimage(&self, key: &Fr) -> Vec<u8> {
        self.0
            .borrow_mut()
            .get_preimage(&key.to_bytes())
            .unwrap_or_default()
            .to_vec()
    }
}

#[derive(Clone)]
pub struct ZkTrieStateDb<DB> {
    storage: NodeDb<DB>,
    trie: Option<ZkTrie<PoseidonHash>>,
}

const MAX_LEVEL: usize = 31 * 8;

impl<DB: TrieDb> ZkTrieStateDb<DB> {
    pub fn new(storage: DB) -> Self {
        Self {
            storage: NodeDb(Rc::new(RefCell::new(storage))),
            trie: None,
        }
    }

    pub fn new_empty(storage: DB) -> Self {
        Self::new_opened(storage, &[0u8; 32])
    }

    pub fn new_opened(storage: DB, root32: &[u8]) -> Self {
        let mut storage = Self::new(storage);
        storage.open(root32);
        storage
    }
}

impl<DB: TrieDb> TrieStorage for ZkTrieStateDb<DB> {
    fn open(&mut self, root32: &[u8]) -> bool {
        if self.trie.is_some() {
            return false;
        }
        self.trie = Some(ZkTrie::new(MAX_LEVEL, Hash::from_bytes(&root32)));
        true
    }

    fn compute_root(&self) -> [u8; 32] {
        self.trie
            .as_ref()
            .map(|trie| trie.hash().clone())
            .unwrap_or(Hash::default())
            .bytes()
    }

    fn get(&self, key: &[u8]) -> Option<Vec<[u8; 32]>> {
        if let Ok(val) = self.trie.as_ref()?.get_data(&self.storage, key) {
            match val {
                TrieData::Node(node) => {
                    let result = node
                        .data()
                        .to_vec()
                        .chunks(32)
                        .map(|val| {
                            let mut bytes = [0u8; 32];
                            bytes.copy_from_slice(val);
                            bytes
                        })
                        .collect::<Vec<_>>();
                    Some(result)
                }
                TrieData::NotFound => None,
            }
        } else {
            None
        }
    }

    fn update(
        &mut self,
        key: &[u8],
        value_flags: u32,
        value: &Vec<[u8; 32]>,
    ) -> Result<(), ExitCode> {
        let trie = self.trie.as_mut().unwrap();
        trie.update(
            &mut self.storage,
            key,
            value_flags,
            value.iter().map(|v| Byte32::from(*v)).collect(),
        )
        .map_err(|_| ExitCode::PersistentStorageError)
    }

    fn remove(&mut self, key: &[u8]) -> Result<(), ExitCode> {
        self.update(key, 0, &vec![POSEIDON_EMPTY.0])
    }

    fn proof(&self, key: &[u8; 32]) -> Option<Vec<Vec<u8>>> {
        let trie = self.trie.as_ref().unwrap();
        match trie.proof(&self.storage, &key[..]) {
            Ok(result) => Some(result),
            Err(_) => None,
        }
    }

    fn get_preimage(&mut self, key: &[u8]) -> Option<Bytes> {
        self.storage.0.borrow_mut().get_preimage(key)
    }

    fn update_preimage(&mut self, key: &[u8], value: Bytes) {
        self.storage.0.borrow_mut().update_preimage(key, value);
    }
}

#[cfg(test)]
mod tests {
    use crate::{storage::TrieStorage, zktrie::ZkTrieStateDb};
    use fluentbase_types::InMemoryAccountDb;
    use std::ops::DerefMut;

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
        let db = InMemoryAccountDb::default();
        // create new zkt
        let mut zkt = ZkTrieStateDb::new_empty(db);
        zkt.update(
            bytes32!("key1"),
            0,
            &vec![*bytes32!("value1"), *bytes32!("value2")],
        )
        .unwrap();
        let root = zkt.compute_root();
        println!("root: {:?}", hex::encode(root));
        // open and read value
        let zkt2 = ZkTrieStateDb::new_opened(zkt.storage.0.borrow_mut().clone(), &root);
        let data = zkt2.get(bytes32!("key1")).unwrap();
        assert_eq!(data[0], *bytes32!("value1"));
        assert_eq!(data[1], *bytes32!("value2"));
    }
}
