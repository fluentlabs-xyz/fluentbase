use crate::{
    storage::{PersistentDatabase, PersistentStorage},
    RuntimeError,
};
use fluentbase_poseidon::Hashable;
use halo2curves::{bn256::Fr, group::ff::PrimeField};
use lazy_static::lazy_static;
use once_cell::race::OnceBox;
use zktrie::{Hash, ZkMemoryDb, ZkTrie};

pub struct ZkTriePersistentStorage<'a, DB> {
    storage: &'a mut DB,
    db: ZkMemoryDb,
    trie: ZkTrie,
}

static FILED_ERROR_READ: &str = "invalid input field";
static FILED_ERROR_OUT: &str = "output field fail";

extern "C" fn hash_scheme(
    a: *const u8,
    b: *const u8,
    domain: *const u8,
    out: *mut u8,
) -> *const i8 {
    use std::slice;
    let a: [u8; 32] =
        TryFrom::try_from(unsafe { slice::from_raw_parts(a, 32) }).expect("length specified");
    let b: [u8; 32] =
        TryFrom::try_from(unsafe { slice::from_raw_parts(b, 32) }).expect("length specified");
    let domain: [u8; 32] =
        TryFrom::try_from(unsafe { slice::from_raw_parts(domain, 32) }).expect("length specified");
    let out = unsafe { slice::from_raw_parts_mut(out, 32) };

    let fa = Fr::from_bytes(&a);
    let fa = if fa.is_some().into() {
        fa.unwrap()
    } else {
        return FILED_ERROR_READ.as_ptr().cast();
    };
    let fb = Fr::from_bytes(&b);
    let fb = if fb.is_some().into() {
        fb.unwrap()
    } else {
        return FILED_ERROR_READ.as_ptr().cast();
    };
    let fdomain = Fr::from_bytes(&domain);
    let fdomain = if fdomain.is_some().into() {
        fdomain.unwrap()
    } else {
        return FILED_ERROR_READ.as_ptr().cast();
    };

    let hasher = Fr::hasher();
    let h = hasher.hash([fa, fb], fdomain);
    let repr_h = h.to_repr();

    if repr_h.len() == 32 {
        out.copy_from_slice(repr_h.as_ref());
        std::ptr::null()
    } else {
        FILED_ERROR_OUT.as_ptr().cast()
    }
}

lazy_static! {
    /// Use this boolean to initialize the hash scheme.
    pub static ref HASH_SCHEME_DONE: bool = {
        zktrie::init_hash_scheme(hash_scheme);
        true
    };
}

impl<'a, DB: PersistentDatabase> ZkTriePersistentStorage<'a, DB> {
    pub fn empty(storage: &'a mut DB) -> Self {
        _ = HASH_SCHEME_DONE.clone();
        let mut db = ZkMemoryDb::new();
        let root = [0u8; 32];
        // we can safely unwrap here because for empty root leave doesn't have to exist
        let trie = db.new_trie(&root).unwrap();
        Self { storage, db, trie }
    }

    pub fn new(
        storage: &'a mut DB,
        root: &[u8; 32],
        nodes: &Vec<Vec<u8>>,
    ) -> Result<Self, RuntimeError> {
        _ = HASH_SCHEME_DONE.clone();
        let mut db = ZkMemoryDb::new();

        for node in nodes.iter() {
            db.add_node_bytes(node.as_slice())
                .map_err(|err| RuntimeError::StorageError(err.to_string()))?;
        }
        let trie = db
            .new_trie(&root)
            .ok_or(RuntimeError::StorageError("can't open zktrie".to_string()))?;
        Ok(Self { storage, db, trie })
    }
}

impl<'a, DB: PersistentDatabase> PersistentStorage for ZkTriePersistentStorage<'a, DB> {
    fn open(&self, _key: &[u8; 32]) -> Result<Self, RuntimeError>
    where
        Self: Sized,
    {
        todo!()
    }

    fn compute_root(&self) -> [u8; 32] {
        self.trie.root()
    }

    fn get(&self, key: &[u8; 32]) -> Option<[u8; 32]> {
        self.trie.get_store(key)
    }

    fn update(&mut self, key: &[u8; 32], value: &[u8; 32]) -> Result<(), RuntimeError> {
        self.trie
            .update_store(key, &value)
            .map_err(|err| RuntimeError::StorageError(err.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::{zktrie::ZkTriePersistentStorage, InMemoryDatabase, PersistentStorage};

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
    fn test_zktrie() {
        let mut db = InMemoryDatabase::default();

        let mut zkt = ZkTriePersistentStorage::empty(&mut db);
        (0..100).for_each(|i| {
            zkt.update(bytes32!(format!("key{}", i)), bytes32!("some_value"))
                .unwrap();
        });
        let root = zkt.compute_root();
        let proof0 = zkt.trie.prove(&root).unwrap();
        println!("{}", proof0.len());
        // let proof1 = zkt.trie.prove(bytes32!("key1")).unwrap();
        // let proof2 = zkt.trie.prove(bytes32!("key7")).unwrap();
        let mut zkt2 = ZkTriePersistentStorage::new(&mut db, &root, &proof0).unwrap();
        assert_eq!(zkt2.get(bytes32!("key1")).unwrap(), *bytes32!("value1"));
    }
}
