use crate::{ZktriePlatformSDK, SDK};
use eth_trie::{EthTrie, MemoryDB, Trie};
use fluentbase_runtime::{
    fetch_balance, fetch_code_hash, fetch_code_size, fetch_nonce, fetch_storage_root, fetch_store,
    get_account_data, get_store_data, update_balance, update_code_hash, update_code_size,
    update_nonce, update_storage_root, update_store, Hash, ZkMemoryDb, ZkTrie, FIELDSIZE,
};
use std::cell::RefCell;
assert_eq!(HASH_SCHEME_DONE, true);

thread_local! {
    static TRIE: RefCell<ZkTrie> = RefCell::new(ZkMemoryDb::new().new_trie(&[0; FIELDSIZE]).unwrap());
}

impl ZktriePlatformSDK for SDK {
    fn zktrie_open() {
        let mut db = ZkMemoryDb::new();
        let root_zero: Hash = [0; FIELDSIZE];
        let zk_trie: ZkTrie = db.new_trie(&root_zero).expect("failed to init new zk trie");
        TRIE.replace(zk_trie);
    }

    fn zktrie_update_nonce(key: &[u8], value: &[u8; FIELDSIZE]) {
        TRIE.with_borrow_mut(|trie| {
            let data = get_account_data(&key, trie);
            let mut data = data.unwrap_or(Default::default());
            let mut field_array: [u8; FIELDSIZE] = [0; FIELDSIZE];
            field_array[FIELDSIZE - value.len()..FIELDSIZE].copy_from_slice(value.as_slice());
            update_nonce(&mut data, &field_array);
            let res = trie.update_account(&key, &data);
            res.expect("failed to update nonce in zktrie")
        });
    }

    fn zktrie_update_balance(key: &[u8], value: &[u8; FIELDSIZE]) {
        TRIE.with_borrow_mut(|trie| {
            let data = get_account_data(&key, trie);
            let mut data = data.unwrap_or(Default::default());
            let mut field_array: [u8; FIELDSIZE] = [0; FIELDSIZE];
            field_array[FIELDSIZE - value.len()..FIELDSIZE].copy_from_slice(value.as_slice());
            update_balance(&mut data, &field_array);
            let res = trie.update_account(&key, &data);
            res.expect("failed to update balance in zktrie")
        });
    }

    fn zktrie_update_storage_root(key: &[u8], value: &[u8; FIELDSIZE]) {
        TRIE.with_borrow_mut(|trie| {
            let data = get_account_data(&key, trie);
            let mut data = data.unwrap_or(Default::default());
            let mut field_array: [u8; FIELDSIZE] = [0; FIELDSIZE];
            field_array[FIELDSIZE - value.len()..FIELDSIZE].copy_from_slice(value.as_slice());
            update_storage_root(&mut data, &field_array);
            let res = trie.update_account(&key, &data);
            res.expect("failed to update storage_root in zktrie")
        });
    }

    fn zktrie_update_code_hash(key: &[u8], value: &[u8; FIELDSIZE]) {
        TRIE.with_borrow_mut(|trie| {
            let data = get_account_data(&key, trie);
            let mut data = data.unwrap_or(Default::default());
            let mut field_array: [u8; FIELDSIZE] = [0; FIELDSIZE];
            field_array[FIELDSIZE - value.len()..FIELDSIZE].copy_from_slice(value.as_slice());
            update_code_hash(&mut data, &field_array);
            let res = trie.update_account(&key, &data);
            res.expect("failed to update code_hash in zktrie")
        });
    }

    fn zktrie_update_code_size(key: &[u8], value: &[u8; FIELDSIZE]) {
        TRIE.with_borrow_mut(|trie| {
            let data = get_account_data(&key, trie);
            let mut data = data.unwrap_or(Default::default());
            let mut field_array: [u8; FIELDSIZE] = [0; FIELDSIZE];
            field_array[FIELDSIZE - value.len()..FIELDSIZE].copy_from_slice(value.as_slice());
            update_code_size(&mut data, &field_array);
            let res = trie.update_account(&key, &data);
            res.expect("failed to update code_size in zktrie")
        });
    }

    fn zktrie_get_nonce(key: &[u8]) -> [u8; FIELDSIZE] {
        TRIE.with_borrow_mut(|trie| {
            let data = get_account_data(&key, trie).expect(&format!("failed to get account data"));

            let data = fetch_nonce(&data);
            data
        })
    }

    fn zktrie_get_balance(key: &[u8]) -> [u8; FIELDSIZE] {
        TRIE.with_borrow_mut(|trie| {
            let data = get_account_data(&key, trie).expect(&format!("failed to get account data"));

            let data = fetch_balance(&data);
            data
        })
    }

    fn zktrie_get_storage_root(key: &[u8]) -> [u8; FIELDSIZE] {
        TRIE.with_borrow_mut(|trie| {
            let data = get_account_data(&key, trie).expect(&format!("failed to get account data"));

            let data = fetch_storage_root(&data);
            data
        })
    }

    fn zktrie_get_code_hash(key: &[u8]) -> [u8; FIELDSIZE] {
        TRIE.with_borrow_mut(|trie| {
            let data = get_account_data(&key, trie).expect(&format!("failed to get account data"));

            let data = fetch_code_hash(&data);
            data
        })
    }

    fn zktrie_get_code_size(key: &[u8]) -> [u8; FIELDSIZE] {
        TRIE.with_borrow_mut(|trie| {
            let data = get_account_data(&key, trie).expect(&format!("failed to get account data"));

            let data = fetch_code_size(&data);
            data
        })
    }

    fn zktrie_update_store(key: &[u8], value: &[u8; FIELDSIZE]) {
        TRIE.with_borrow_mut(|trie| {
            let data = get_store_data(&key, trie);
            let mut data = data.unwrap_or(Default::default());
            let mut field_array: [u8; FIELDSIZE] = [0; FIELDSIZE];
            field_array[FIELDSIZE - value.len()..FIELDSIZE].copy_from_slice(value.as_slice());
            update_store(&mut data, &field_array);
            let res = trie.update_store(&key, &data);
            res.expect("failed to update store in zktrie")
        });
    }

    fn zktrie_get_store(key: &[u8]) -> [u8; FIELDSIZE] {
        TRIE.with_borrow_mut(|trie| {
            let data = get_store_data(&key, trie).expect(&format!("failed to get account data"));

            let data = fetch_store(&data);
            data
        })
    }
}
