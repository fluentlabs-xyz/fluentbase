// use ethereum_types::H256;
// use hashdb::Hasher;
// use plain_hasher::PlainHasher;
// use tiny_keccak::Keccak;
//
// /// Concrete `Hasher` impl for the Keccak-256 hash
// #[derive(Default, Debug, Clone, PartialEq)]
// pub struct KeccakHasher;
// impl Hasher for KeccakHasher {
//     type Out = H256;
//     type StdHasher = PlainHasher;
//     const LENGTH: usize = 32;
//     fn hash(x: &[u8]) -> Self::Out {
//         let mut out = [0; Self::LENGTH];
//         Keccak::keccak256(x, &mut out);
//         out.into()
//     }
// }

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
    use crate::mpt_helpers::{KeyValue, KeysValues};
    use rlp::Decodable;

    #[test]
    pub fn encode_decode_empty_struct_test() {
        let kvs = KeysValues(vec![]);

        let enc = rlp::encode(&kvs);
        let dec = rlp::decode::<KeysValues>(&enc).unwrap();

        assert_eq!(kvs.0.len(), 0);
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
