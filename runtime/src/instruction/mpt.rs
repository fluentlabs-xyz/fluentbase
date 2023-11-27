use crate::{instruction::exported_memory_vec, RuntimeContext};
use eth_trie::{EthTrie, MemoryDB, Trie};
use fluentbase_rwasm::Caller;
use fluentbase_rwasm_core::common::Trap;
use std::{cell::RefCell, collections::HashMap, hint::unreachable_unchecked, rc::Rc, sync::Arc};

type TrieId = i32;
const TRIE_ID_DEFAULT: i32 = 1;
thread_local! {
    static LAST_TRIE_ID: RefCell<TrieId> = RefCell::new(1);
    static TRIES: RefCell<HashMap<TrieId, Rc<RefCell<EthTrie<MemoryDB>>>>> = RefCell::new(HashMap::new());
}

pub(crate) fn mpt_open(
    _caller: Caller<'_, RuntimeContext>,
    // rlp_offset: i32,
    // rlp_len: i32,
) -> Result<(), Trap> {
    let trie_id;
    trie_id = LAST_TRIE_ID.take();
    if trie_id != TRIE_ID_DEFAULT {
        return Err(Trap::new("only one trie allowed at the moment"));
    }

    let trie = EthTrie::new(Arc::new(MemoryDB::new(true)));

    // let rlp_data = exported_memory_vec(&mut caller, rlp_offset as usize, rlp_len as usize);
    // let keys_values = rlp::decode::<KeysValues>(&rlp_data).unwrap();
    // for kv in &keys_values.0 {
    //     trie.insert(kv.key.as_slice(), kv.value.as_slice())
    //         .map_err(|e| {
    //             Trap::new(format!(
    //                 "failed to insert kv into the trie: {}",
    //                 e.to_string()
    //             ))
    //         })?;
    // }

    TRIES.with_borrow_mut(|m| m.insert(trie_id, Rc::new(RefCell::new(trie))));
    // LAST_TRIE_ID.with_borrow_mut(|v| *v += 1);

    Ok(())
}

pub(crate) fn mpt_get_trie(id: &TrieId) -> Result<Rc<RefCell<EthTrie<MemoryDB>>>, Trap> {
    TRIES.with(|t| {
        let tries = t.take();
        let v = tries.get(id).clone();
        if let Some(t) = v {
            return Ok((*t).clone());
        }
        Err(Trap::new("not found"))
    })
}

pub(crate) fn mpt_update(
    mut caller: Caller<'_, RuntimeContext>,
    key_offset: i32,
    key_len: i32,
    value_offset: i32,
    value_len: i32,
) -> Result<(), Trap> {
    let trie_id;
    trie_id = LAST_TRIE_ID.take();
    if trie_id != TRIE_ID_DEFAULT {
        return Err(Trap::new("only 1 trie allowed"));
    }

    let mut trie = EthTrie::new(Arc::new(MemoryDB::new(true)));

    let key_data = exported_memory_vec(&mut caller, key_offset as usize, key_len as usize);
    let value_data = exported_memory_vec(&mut caller, value_offset as usize, value_len as usize);
    // let kv = rlp::decode::<KeyValue>(&rlp_data).unwrap();
    trie.insert(key_data.as_slice(), value_data.as_slice())
        .map_err(|e| {
            Trap::new(format!(
                "failed to insert kv into the trie: {}",
                e.to_string()
            ))
        })?;

    TRIES.with_borrow_mut(|m| m.insert(trie_id, Rc::new(RefCell::new(trie))));
    // LAST_TRIE_ID.with_borrow_mut(|v| *v += 1);

    Ok(())
}

pub(crate) fn mpt_get(
    mut caller: Caller<'_, RuntimeContext>,
    key_offset: i32,
    key_len: i32,
    output_offset: i32,
) -> Result<i32, Trap> {
    let trie_id;
    trie_id = LAST_TRIE_ID.take();
    if trie_id != TRIE_ID_DEFAULT {
        return Err(Trap::new("only one trie allowed at the moment"));
    }

    let key_data = exported_memory_vec(&mut caller, key_offset as usize, key_len as usize);

    let trie = mpt_get_trie(&trie_id)?;

    let res = trie
        .borrow_mut()
        .get(&key_data)
        .map_err(|e| Trap::new(format!("failed to get value by the key: {}", e.to_string())))?
        .ok_or(Trap::new(format!("there is no value by provided key")))?;

    caller.write_memory(output_offset as usize, res.as_slice());

    Ok(res.len() as i32)
}

pub(crate) fn mpt_get_root(
    mut caller: Caller<'_, RuntimeContext>,
    output_offset: i32,
) -> Result<i32, Trap> {
    let trie_id;
    trie_id = LAST_TRIE_ID.take();
    if trie_id != TRIE_ID_DEFAULT {
        return Err(Trap::new("only 1 trie allowed"));
    }

    let trie = mpt_get_trie(&trie_id)?;

    let res = trie
        .borrow_mut()
        .root_hash()
        .map_err(|e| Trap::new(format!("failed to get root: {}", e.to_string())))?;

    caller.write_memory(output_offset as usize, res.as_bytes());

    Ok(res.0.len() as i32)
}

use rlp::{Decodable, DecoderError, Encodable, Rlp, RlpStream};

#[derive(Default, Debug, Clone)]
pub struct KeyValue {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

#[derive(Default, Debug, Clone)]
pub struct KeysValues(pub Vec<KeyValue>);

impl Encodable for KeyValue {
    fn rlp_append(&self, s: &mut RlpStream) {
        s.begin_list(2);
        s.append(&self.key);
        s.append(&self.value);
    }
}

impl Decodable for KeyValue {
    fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
        if !rlp.is_list() {
            return Err(DecoderError::RlpExpectedToBeList);
        }
        Ok(KeyValue {
            key: rlp.at_with_offset(0)?.0.data()?.to_vec(),
            value: rlp.at_with_offset(1)?.0.data()?.to_vec(),
        })
    }
}

impl Encodable for KeysValues {
    fn rlp_append(&self, s: &mut RlpStream) {
        s.begin_list(self.0.len());
        for kv in &self.0 {
            <KeyValue as Encodable>::rlp_append(kv, s);
        }
    }
}

impl Decodable for KeysValues {
    fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
        if !rlp.is_list() {
            return Err(DecoderError::RlpExpectedToBeList);
        }
        let mut v = vec![];
        for pair_rlp in rlp.iter() {
            v.push(<KeyValue as Decodable>::decode(&pair_rlp)?);
        }
        Ok(KeysValues(v))
    }
}

#[cfg(test)]
mod test {
    use crate::instruction::{KeyValue, KeysValues};
    use rlp::Decodable;

    #[test]
    pub fn encode_decode_empty_struct_test() {
        let kvs = KeysValues(vec![]);

        let enc = rlp::encode(&kvs);
        let dec = rlp::decode::<KeysValues>(&enc).unwrap();

        assert_eq!(kvs.0.len(), 0);
        assert_eq!(dec.0.len(), 0);
    }

    #[test]
    pub fn encode_decode_non_empty_struct_test() {
        let k_v = "key's value".as_bytes().to_vec();
        let v_v = "value's value".as_bytes().to_vec();
        let mut kvs = KeysValues(vec![]);
        let count = 2;
        for i in 0..count {
            let mut key = vec![];
            key.extend(&k_v);
            key.push(i);
            let mut value = vec![];
            value.extend(&v_v);
            value.push(i);
            kvs.0.push(KeyValue { key, value });
        }

        let enc = rlp::encode(&kvs);
        let dec = rlp::decode::<KeysValues>(&enc).unwrap();

        for i in 0..count {
            let mut key = vec![];
            key.extend(&k_v);
            key.push(i);
            let mut value = vec![];
            value.extend(&v_v);
            value.push(i);
            assert_eq!(dec.0[i as usize].key, key);
            assert_eq!(dec.0[i as usize].value, value);
        }
    }

    #[test]
    pub fn encode_some_data_test() {
        let k_v = "key".as_bytes().to_vec();
        println!("k_v: {:?}", k_v);
        let v_v = "value".as_bytes().to_vec();
        println!("v_v: {:?}", v_v);
        let kvs = KeysValues(vec![KeyValue {
            key: k_v,
            value: v_v,
        }]);

        let enc = rlp::encode(&kvs).to_vec();

        let enc_vec = [203, 202, 131, 107, 101, 121, 133, 118, 97, 108, 117, 101];
        assert_eq!(enc, enc_vec);
    }
}
