use crate::{MptPlatformSDK, SDK};

// thread_local! {
//     static TRIE: RefCell<EthTrie<MemoryDB>> =
// RefCell::new(EthTrie::new(Arc::new(MemoryDB::new(true)))); }

impl MptPlatformSDK for SDK {
    fn mpt_open() {
        unreachable!("not implemented")
        // TRIE.replace(EthTrie::new(Arc::new(MemoryDB::new(true))));
    }

    fn mpt_update(key: &[u8], value: &[u8]) {
        unreachable!("not implemented")
        // TRIE.with_borrow_mut(|trie| trie.insert(key, value).unwrap());
    }

    fn mpt_get(key: &[u8], output: &mut [u8]) -> i32 {
        unreachable!("not implemented")
        // TRIE.with_borrow(|trie| {
        //     if let Some(value) = trie.get(key).unwrap() {
        //         output.copy_from_slice(value.as_slice())
        //     }
        // });
        // 0
    }

    fn mpt_root(output: &mut [u8]) -> i32 {
        unreachable!("not implemented")
        // TRIE.with_borrow_mut(|trie| {
        //     let trie_root = trie.root_hash().unwrap();
        //     output.copy_from_slice(trie_root.as_bytes());
        // });
        // 0
    }
}
