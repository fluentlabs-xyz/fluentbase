use crate::storage::TrieDb;
use fluentbase_types::{AccountDb, ExitCode};
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
use std::{cell::RefCell, sync::Arc};

struct NodeDb<'a, DB>(RefCell<&'a mut DB>);

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

impl<'a, DB: AccountDb> Database for NodeDb<'a, DB> {
    type Node = Node<PoseidonHash>;

    fn get_node(&self, key: &Hash) -> Result<Option<Arc<Self::Node>>, Error> {
        match self.0.borrow_mut().get_node(key.raw_bytes()) {
            Some(value) => Ok(Some(Arc::new(Node::from_bytes(&value)?))),
            None => Ok(None),
        }
    }

    fn update_node(&mut self, node: Self::Node) -> Result<Arc<Self::Node>, Error> {
        self.0
            .borrow_mut()
            .update_node(node.hash().raw_bytes(), &node.canonical_value());
        Ok(Arc::new(node))
    }
}

impl<'a, DB: AccountDb> PreimageDatabase for NodeDb<'a, DB> {
    fn update_preimage(&mut self, preimage: &[u8], hash_field: &Fr) {
        self.0
            .borrow_mut()
            .update_preimage(&hash_field.to_bytes(), &preimage.to_vec());
    }

    fn preimage(&self, key: &Fr) -> Vec<u8> {
        self.0
            .borrow_mut()
            .get_preimage(&key.to_bytes())
            .unwrap_or_default()
    }
}

pub struct ZkTrieStateDb<'a, DB> {
    storage: NodeDb<'a, DB>,
    trie: Option<ZkTrie<PoseidonHash>>,
}

const MAX_LEVEL: usize = 31 * 8;

impl<'a, DB: AccountDb> ZkTrieStateDb<'a, DB> {
    pub fn new(storage: &'a mut DB) -> Self {
        Self {
            storage: NodeDb(RefCell::new(storage)),
            trie: None,
        }
    }

    pub fn new_empty(storage: &'a mut DB) -> Self {
        Self::new_opened(storage, &[0u8; 32])
    }

    pub fn new_opened(storage: &'a mut DB, root32: &[u8]) -> Self {
        let mut storage = Self::new(storage);
        storage.open(root32);
        storage
    }
}

impl<'a, DB: AccountDb> TrieDb for ZkTrieStateDb<'a, DB> {
    fn open(&mut self, root32: &[u8]) {
        self.trie = Some(ZkTrie::new(MAX_LEVEL, Hash::from_bytes(&root32)));
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

    fn proof(&self, key: &[u8; 32]) -> Option<Vec<Vec<u8>>> {
        let trie = self.trie.as_ref().unwrap();
        match trie.proof(&self.storage, &key[..]) {
            Ok(result) => Some(result),
            Err(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::{zktrie::ZkTrieStateDb, TrieDb};
    use fluentbase_types::InMemoryAccountDb;

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
        let mut db = InMemoryAccountDb::default();
        // create new zkt
        let mut zkt = ZkTrieStateDb::new_empty(&mut db);
        zkt.update(
            bytes32!("key1"),
            0,
            &vec![*bytes32!("value1"), *bytes32!("value2")],
        )
        .unwrap();
        let root = zkt.compute_root();
        println!("root: {:?}", hex::encode(root));
        // open and read value
        let zkt2 = ZkTrieStateDb::new_opened(&mut db, &root);
        let data = zkt2.get(bytes32!("key1")).unwrap();
        assert_eq!(data[0], *bytes32!("value1"));
        assert_eq!(data[1], *bytes32!("value2"));
    }
}
