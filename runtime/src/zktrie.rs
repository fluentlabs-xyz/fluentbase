use crate::{instruction::exported_memory_vec, RuntimeContext};
use fluentbase_rwasm::{common::Trap, Caller};
use halo2_proofs::halo2curves::{bn256::Fr, group::ff::PrimeField};
use lazy_static::lazy_static;
use poseidon_circuit::hash::Hashable;
use std::{
    cell::{RefCell, RefMut},
    collections::HashMap,
    rc::Rc,
};
use zktrie::{AccountData, StoreData, ZkMemoryDb, ZkTrie, FIELDSIZE};

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

    let h = Fr::hash_with_domain([fa, fb], fdomain);
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

type TrieId = i32;
thread_local! {
    static ZK_MEMORY_DB: RefCell<ZkMemoryDb> = RefCell::new(ZkMemoryDb::new());
    static LAST_TRIE_ID: RefCell<TrieId> = RefCell::new(0);
    static TRIES: RefCell<HashMap<TrieId, Rc<RefCell<ZkTrie>>>> = RefCell::new(HashMap::new());
}

pub(crate) fn zktrie_new_trie(
    mut _caller: Option<Caller<'_, RuntimeContext>>,
    // root: &Hash,
) -> Result<TrieId, Trap> {
    let root = [0; FIELDSIZE];
    ZK_MEMORY_DB.with(|db| {
        let t = db.borrow_mut().new_trie(&root);
        if let Some(t) = t {
            let trie_id = LAST_TRIE_ID.take();
            TRIES.with_borrow_mut(|m| m.insert(trie_id, Rc::new(RefCell::new(t))));
            LAST_TRIE_ID.with_borrow_mut(|v| {
                *v = *v + 1;
            });
            return Ok(trie_id);
        }
        Err(Trap::new("failed to init new trie"))
    })
}

pub(crate) fn zktrie_get(id: &TrieId) -> Result<Rc<RefCell<ZkTrie>>, Trap> {
    TRIES.with(|t| {
        let tries = t.take();
        let v = tries.get(id).clone();
        if let Some(t) = v {
            return Ok(t.clone());
        }
        Err(Trap::new("not found"))
    })
}

fn get_or_default_account_data(key: &[u8], trie: RefMut<ZkTrie>) -> AccountData {
    let acc_data = trie.get_account(&key);
    if let Some(ad) = acc_data {
        ad
    } else {
        AccountData::default()
    }
}

fn get_or_default_store_data(key: &[u8], trie: RefMut<ZkTrie>) -> StoreData {
    let acc_data = trie.get_store(&key);
    if let Some(ad) = acc_data {
        ad
    } else {
        StoreData::default()
    }
}

macro_rules! impl_account_update {
    ($fn_name:ident, $extractor:ident, $field_updater:ident, $trie_updater:ident, ) => {
        pub(crate) fn $fn_name(
            mut caller: Caller<'_, RuntimeContext>,
            trie_id: TrieId,
            key_offset: i32,
            value_offset: i32,
        ) -> Result<(), Trap> {
            let key = exported_memory_vec(&mut caller, key_offset as usize, FIELDSIZE);
            let nonce = exported_memory_vec(&mut caller, value_offset as usize, FIELDSIZE);

            let trie = zktrie_get(&trie_id)?;
            let mut data = $extractor(&key, trie.borrow_mut());
            let mut nonce_array: [u8; FIELDSIZE] = [0; FIELDSIZE];
            // TODO BE or LE?
            nonce_array[FIELDSIZE - nonce.len()..FIELDSIZE].copy_from_slice(nonce.as_slice());
            $field_updater(&mut data, &nonce_array);
            let res = trie.borrow_mut().$trie_updater(&key, &data);
            if res.is_err() {
                return Err(Trap::new(format!(
                    "failed to update account nonce: {}",
                    res.err().unwrap().to_string()
                )));
            }

            Ok(())
        }
    };
}
impl_account_update!(
    zktrie_update_nonce,
    get_or_default_account_data,
    update_nonce,
    update_account,
);
impl_account_update!(
    zktrie_update_balance,
    get_or_default_account_data,
    update_balance,
    update_account,
);
impl_account_update!(
    zktrie_update_storage_root,
    get_or_default_account_data,
    update_storage_root,
    update_account,
);
impl_account_update!(
    zktrie_update_code_hash,
    get_or_default_account_data,
    update_code_hash,
    update_account,
);
impl_account_update!(
    zktrie_update_code_size,
    get_or_default_account_data,
    update_code_size,
    update_account,
);

impl_account_update!(
    zktrie_update_store,
    get_or_default_store_data,
    update_store,
    update_store,
);

pub(crate) fn zktrie_update_nonce_(
    // mut caller: Caller<'_, RuntimeContext>,
    trie_id: TrieId,
    // key_offset: i32,
    // value_offset: i32,
    key: Vec<u8>,
    nonce: Vec<u8>,
) -> Result<(), Trap> {
    // let key = exported_memory_vec(&mut caller, key_offset as usize, FIELDSIZE);
    // let nonce = exported_memory_vec(&mut caller, value_offset as usize, FIELDSIZE);

    let trie = zktrie_get(&trie_id)?;
    let mut acc_data = get_or_default_account_data(&key, trie.borrow_mut());
    let mut nonce_array: [u8; FIELDSIZE] = [0; FIELDSIZE];
    // TODO BE or LE?
    nonce_array[FIELDSIZE - nonce.len()..FIELDSIZE].copy_from_slice(nonce.as_slice());
    update_nonce(&mut acc_data, &nonce_array);
    let res = trie.borrow_mut().update_account(&key, &acc_data);
    if res.is_err() {
        return Err(Trap::new(format!(
            "failed to update account nonce: {}",
            res.err().unwrap().to_string()
        )));
    }

    Ok(())
}

pub(crate) fn zktrie_update_store_(
    // mut caller: Caller<'_, RuntimeContext>,
    trie_id: TrieId,
    // key_offset: i32,
    // value_offset: i32,
    key: Vec<u8>,
    nonce: Vec<u8>,
) -> Result<(), Trap> {
    // let key = exported_memory_vec(&mut caller, key_offset as usize, FIELDSIZE);
    // let nonce = exported_memory_vec(&mut caller, value_offset as usize, FIELDSIZE);

    let trie = zktrie_get(&trie_id)?;
    let mut data = get_or_default_store_data(&key, trie.borrow_mut());
    let mut nonce_array: [u8; FIELDSIZE] = [0; FIELDSIZE];
    // TODO BE or LE?
    nonce_array[FIELDSIZE - nonce.len()..FIELDSIZE].copy_from_slice(nonce.as_slice());
    update_store(&mut data, &nonce_array);
    let res = trie.borrow_mut().update_store(&key, &data);
    if res.is_err() {
        return Err(Trap::new(format!(
            "failed to update account nonce: {}",
            res.err().unwrap().to_string()
        )));
    }

    Ok(())
}

fn update_nonce(data: &mut AccountData, v: &[u8; FIELDSIZE]) {
    data[0] = *v;
}

fn update_balance(data: &mut AccountData, v: &[u8; FIELDSIZE]) {
    data[1] = *v;
}

fn update_storage_root(data: &mut AccountData, v: &[u8; FIELDSIZE]) {
    data[2] = *v;
}

fn update_code_hash(data: &mut AccountData, v: &[u8; FIELDSIZE]) {
    data[3] = *v;
}

fn update_code_size(data: &mut AccountData, v: &[u8; FIELDSIZE]) {
    data[4] = *v;
}

fn update_store(data: &mut StoreData, v: &[u8; FIELDSIZE]) {
    *data = *v;
}

#[cfg(test)]
mod zktrie_tests {
    use crate::zktrie::{zktrie_new_trie, zktrie_update_nonce_};

    #[test]
    pub fn test() {
        let trie_id = zktrie_new_trie(None).unwrap();
        let res = zktrie_update_nonce_(trie_id, [1; 32].to_vec(), vec![12]);
        res.unwrap();
    }
}
