use crate::{
    complex_types::RuntimeError,
    storage::{KeyValueDb, TrieDb},
};
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
use std::sync::Arc;

struct NodeDb<'a, DB>(&'a mut DB);

const STORAGE_PREFIX_NODE: u8 = 0x01;
const STORAGE_PREFIX_PREIMAGE: u8 = 0x02;

impl<'a, DB: KeyValueDb> Database for NodeDb<'a, DB> {
    type Node = Node<PoseidonHash>;

    fn get_node(&self, key: &Hash) -> Result<Option<Arc<Self::Node>>, Error> {
        let storage_key = {
            let mut storage_key = [0u8; 33];
            storage_key[0] = STORAGE_PREFIX_NODE;
            storage_key[1..].copy_from_slice(key.raw_bytes());
            storage_key
        };
        match self.0.get(&storage_key[..]) {
            Some(value) => Ok(Some(Arc::new(Node::from_bytes(value.as_slice())?))),
            None => Ok(None),
        }
    }

    fn update_node(&mut self, node: Self::Node) -> Result<Arc<Self::Node>, Error> {
        let storage_key = {
            let mut storage_key = [0u8; 33];
            storage_key[0] = STORAGE_PREFIX_NODE;
            storage_key[1..].copy_from_slice(node.hash().raw_bytes());
            storage_key
        };
        self.0.put(&storage_key[..], &node.canonical_value());
        Ok(Arc::new(node))
    }
}

impl<'a, DB: KeyValueDb> PreimageDatabase for NodeDb<'a, DB> {
    fn update_preimage(&mut self, preimage: &[u8], hash_field: &Fr) {
        let storage_key = {
            let mut storage_key = [0u8; 33];
            storage_key[0] = STORAGE_PREFIX_PREIMAGE;
            storage_key[1..].copy_from_slice(&hash_field.to_bytes());
            storage_key
        };
        self.0.put(&storage_key[..], &preimage.to_vec());
    }

    fn preimage(&self, key: &Fr) -> Vec<u8> {
        let storage_key = {
            let mut storage_key = [0u8; 33];
            storage_key[0] = STORAGE_PREFIX_PREIMAGE;
            storage_key[1..].copy_from_slice(&key.to_bytes());
            storage_key
        };
        self.0.get(&storage_key[..]).unwrap_or_default()
    }
}

pub struct ZkTriePersistentStorage<'a, DB> {
    storage: NodeDb<'a, DB>,
    trie: ZkTrie<PoseidonHash>,
}

const MAX_LEVEL: usize = 31 * 8;

impl<'a, DB: KeyValueDb> ZkTriePersistentStorage<'a, DB> {
    pub fn empty(storage: &'a mut DB) -> Self {
        let root = [0u8; 32];
        Self {
            storage: NodeDb(storage),
            trie: ZkTrie::new(MAX_LEVEL, Hash::from_bytes(&root)),
        }
    }

    pub fn new(storage: &'a mut DB, root: &[u8; 32]) -> Result<Self, RuntimeError> {
        Ok(Self {
            storage: NodeDb(storage),
            trie: ZkTrie::new(MAX_LEVEL, Hash::from_bytes(&root[..])),
        })
    }
}

impl<'a, DB: KeyValueDb> TrieDb for ZkTriePersistentStorage<'a, DB> {
    fn open(&self, _key: &[u8; 32]) -> Result<Self, RuntimeError>
    where
        Self: Sized,
    {
        todo!()
    }

    fn compute_root(&self) -> [u8; 32] {
        self.trie.hash().bytes()
    }

    fn get(&self, key: &[u8; 32]) -> Option<Vec<u8>> {
        if let Ok(val) = self.trie.get_data(&self.storage, &key[..]) {
            match val {
                TrieData::Node(node) => Some(node.data().to_vec()),
                TrieData::NotFound => None,
            }
        } else {
            None
        }
    }

    fn update(
        &mut self,
        key: &[u8; 32],
        value_flags: u32,
        value: &Vec<[u8; 32]>,
    ) -> Result<(), RuntimeError> {
        self.trie
            .update(
                &mut self.storage,
                &key[..],
                value_flags,
                value.iter().map(|v| Byte32::from(*v)).collect(),
            )
            .map_err(|err| RuntimeError::StorageError(format!("can't update value ({:?})", err)))
    }

    fn proof(&self, key: &[u8; 32]) -> Option<Vec<Vec<u8>>> {
        match self.trie.proof(&self.storage, &key[..]) {
            Ok(result) => Some(result),
            Err(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::{zktrie::ZkTriePersistentStorage, InMemoryDb, TrieDb};

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
        let mut db = InMemoryDb::default();
        // create new zkt
        let mut zkt = ZkTriePersistentStorage::empty(&mut db);
        zkt.update(
            bytes32!("key1"),
            0,
            &vec![*bytes32!("value1"), *bytes32!("value2")],
        )
        .unwrap();
        let root = zkt.compute_root();
        println!("root: {:?}", hex::encode(root));
        // open and read value
        let zkt2 = ZkTriePersistentStorage::new(&mut db, &root).unwrap();
        let data = zkt2.get(bytes32!("key1")).unwrap();
        assert_eq!(data[0..32], *bytes32!("value1"));
        assert_eq!(data[32..64], *bytes32!("value2"));
    }
}
