use super::bytes::Bytes;
use crate::hash::{keccak, KECCAK_EMPTY_LIST_RLP, KECCAK_NULL_RLP};
use ethereum_types::{Address, Bloom, H160, H256, U256, U64};
use hex::FromHexError;
use rlp::*;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{borrow::BorrowMut, cell::RefCell, cmp};
use thiserror::Error;

/// Semantic boolean for when a seal/signature is included.
pub enum Seal {
    /// The seal/signature is included.
    With,
    /// The seal/signature is not included.
    Without,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Header {
    /// Parent hash.
    parent_hash: H256,
    #[serde(rename = "sha3Uncles")]
    uncle_hash: H256,

    #[serde(rename = "miner")]
    coinbase: Address,

    /// State root.
    state_root: H256,
    /// Transactions root.
    transactions_root: H256,
    /// Block receipts root.
    receipts_root: H256,

    /// Block bloom.
    logs_bloom: Bloom,

    /// Block difficulty.
    difficulty: U256,
    /// Block number.
    number: U64,

    /// Gas used for contracts execution.
    gas_used: U256,
    /// Block gas limit.
    gas_limit: U256,

    /// Block timestamp.
    timestamp: U64,

    #[serde(serialize_with = "se_hex")]
    #[serde(deserialize_with = "de_hex_to_vec_u8")]
    extra_data: Vec<u8>,

    nonce: U256,
    /// Vector of post-RLP-encoded fields.
    // seal: Vec<Bytes>,
    mix_hash: H256,
    /// The memoized hash of the RLP representation *including* the seal fields.
    #[serde(skip)]
    hash: RefCell<Option<H256>>,
    // /// The memoized hash of the RLP representation *without* the seal fields.
    // bare_hash: RefCell<Option<H256>>,
}

/// Encode hex with 0x prefix
pub fn hex_encode<T: AsRef<[u8]>>(data: T) -> String {
    format!("0x{}", hex::encode(data))
}

/// An error from a byte utils operation.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum ByteUtilsError {
    #[error("Hex string starts with {first_two}, expected 0x")]
    WrongPrefix { first_two: String },

    #[error("Unable to decode hex string {data} due to {source}")]
    HexDecode { source: FromHexError, data: String },

    #[error("Hex string is '{data}', expected to start with 0x")]
    NoPrefix { data: String },
}

/// Decode hex with 0x prefix
pub fn hex_decode(data: &str) -> Result<Vec<u8>, ByteUtilsError> {
    let first_two = data.get(..2).ok_or_else(|| ByteUtilsError::NoPrefix {
        data: data.to_string(),
    })?;

    if first_two != "0x" {
        return Err(ByteUtilsError::WrongPrefix {
            first_two: first_two.to_string(),
        });
    }

    let post_prefix = data.get(2..).unwrap_or("");

    hex::decode(post_prefix).map_err(|e| ByteUtilsError::HexDecode {
        source: e,
        data: data.to_string(),
    })
}

fn se_hex<S>(value: &[u8], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&hex_encode(value))
}

fn de_hex_to_vec_u8<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    let result: String = Deserialize::deserialize(deserializer)?;
    hex_decode(&result).map_err(serde::de::Error::custom)
}

fn de_hex_to_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let result: String = Deserialize::deserialize(deserializer)?;
    let result = result.trim_start_matches("0x");
    u64::from_str_radix(result, 16).map_err(serde::de::Error::custom)
}

impl PartialEq for Header {
    fn eq(&self, c: &Header) -> bool {
        self.parent_hash == c.parent_hash
            && self.timestamp == c.timestamp
            && self.number == c.number
            && self.coinbase == c.coinbase
            && self.transactions_root == c.transactions_root
            && self.uncle_hash == c.uncle_hash
            && self.extra_data == c.extra_data
            && self.state_root == c.state_root
            && self.receipts_root == c.receipts_root
            && self.logs_bloom == c.logs_bloom
            && self.gas_used == c.gas_used
            && self.gas_limit == c.gas_limit
            && self.difficulty == c.difficulty
    }
}

impl Default for Header {
    fn default() -> Self {
        Header {
            parent_hash: H256::default(),
            timestamp: U64::default(),
            number: U64::default(),

            transactions_root: KECCAK_NULL_RLP,
            uncle_hash: KECCAK_EMPTY_LIST_RLP,
            extra_data: vec![],

            state_root: KECCAK_NULL_RLP,
            receipts_root: KECCAK_NULL_RLP,
            logs_bloom: Bloom::default(),
            gas_used: U256::default(),
            gas_limit: U256::default(),

            difficulty: U256::default(),
            coinbase: H160::default(),
            nonce: U256::default(),
            mix_hash: H256::default(),
            hash: RefCell::new(Some(H256::default())),
        }
    }
}

impl Header {
    /// Create a new, default-valued, header.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the parent_hash field of the header.
    pub fn parent_hash(&self) -> &H256 {
        &self.parent_hash
    }
    /// Get the timestamp field of the header.
    pub fn timestamp(&self) -> U64 {
        self.timestamp
    }
    /// Get the number field of the header.
    pub fn number(&self) -> U64 {
        self.number
    }
    /// Get the author field of the header.
    pub fn coinbase(&self) -> &Address {
        &self.coinbase
    }

    /// Get the extra data field of the header.
    pub fn extra_data(&self) {
        let xx = hex::encode(&self.extra_data);
        println!("{:?}", xx.as_str())
    }

    /// Get the state root field of the header.
    pub fn state_root(&self) -> &H256 {
        &self.state_root
    }
    /// Get the receipts root field of the header.
    pub fn receipts_root(&self) -> &H256 {
        &self.receipts_root
    }
    /// Get the logs bloom field of the header.
    pub fn logs_bloom(&self) -> &Bloom {
        &self.logs_bloom
    }
    /// Get the transactions root field of the header.
    pub fn transactions_root(&self) -> &H256 {
        &self.transactions_root
    }
    /// Get the uncles hash field of the header.
    pub fn uncle_hash(&self) -> &H256 {
        &self.uncle_hash
    }
    /// Get the gas used field of the header.
    pub fn gas_used(&self) -> &U256 {
        &self.gas_used
    }
    /// Get the gas limit field of the header.
    pub fn gas_limit(&self) -> &U256 {
        &self.gas_limit
    }

    /// Get the difficulty field of the header.
    pub fn difficulty(&self) -> &U256 {
        &self.difficulty
    }

    // /// Get the seal field with RLP-decoded values as bytes.
    // pub fn decode_seal<'a, T: ::std::iter::FromIterator<&'a [u8]>>(
    //     &'a self,
    // ) -> Result<T, DecoderError> { self.seal.iter().map(|rlp| Rlp::new(rlp).data()).collect()
    // }

    /// Set the number field of the header.
    pub fn set_parent_hash(&mut self, a: H256) {
        self.parent_hash = a;
        self.note_dirty();
    }
    /// Set the uncles hash field of the header.
    pub fn set_uncles_hash(&mut self, a: H256) {
        self.uncle_hash = a;
        self.note_dirty();
    }
    /// Set the state root field of the header.
    pub fn set_state_root(&mut self, a: H256) {
        self.state_root = a;
        self.note_dirty();
    }
    /// Set the transactions root field of the header.
    pub fn set_transactions_root(&mut self, a: H256) {
        self.transactions_root = a;
        self.note_dirty()
    }
    /// Set the receipts root field of the header.
    pub fn set_receipts_root(&mut self, a: H256) {
        self.receipts_root = a;
        self.note_dirty()
    }
    /// Set the log bloom field of the header.
    pub fn set_log_bloom(&mut self, a: Bloom) {
        self.logs_bloom = a;
        self.note_dirty()
    }
    /// Set the timestamp field of the header.
    pub fn set_timestamp(&mut self, a: U64) {
        self.timestamp = a;
        self.note_dirty();
    }
    // /// Set the timestamp field of the header to the current time.
    // pub fn set_timestamp_now(&mut self, but_later_than: u64) {
    //     self.timestamp = cmp::max(get_time().sec as u64, but_later_than + 1);
    //     self.note_dirty();
    // }
    /// Set the number field of the header.
    pub fn set_number(&mut self, a: U64) {
        self.number = a;
        self.note_dirty();
    }
    /// Set the coinbase(miner) field of the header.
    pub fn set_miner(&mut self, a: Address) {
        if a != self.coinbase {
            self.coinbase = a;
            self.note_dirty();
        }
    }
    // /// Set the extra data field of the header.
    // pub fn set_extra_data(&mut self, a: Bytes) {
    //     if a != self.extra_data {
    //         self.extra_data = a;
    //         self.note_dirty();
    //     }
    // }
    /// Set the gas used field of the header.
    pub fn set_gas_used(&mut self, a: U256) {
        self.gas_used = a;
        self.note_dirty();
    }
    /// Set the gas limit field of the header.
    pub fn set_gas_limit(&mut self, a: U256) {
        self.gas_limit = a;
        self.note_dirty();
    }
    /// Set the difficulty field of the header.
    pub fn set_difficulty(&mut self, a: U256) {
        self.difficulty = a;
        self.note_dirty();
    }
    /// Get the hash of this header (keccak of the RLP).
    pub fn hash(&self) -> H256 {
        let mut hash = self.hash.borrow_mut();
        match &mut *hash {
            &mut Some(ref h) => h.clone(),
            hash @ &mut None => {
                let h = self.rlp_sha3(Seal::Without);
                *hash = Some(h.clone());
                h
            }
        }
    }

    /// Get the SHA3 (Keccak) of this header, optionally `with_seal`.
    pub fn rlp_sha3(&self, with_seal: Seal) -> H256 {
        keccak(self.rlp(with_seal).0)
    }

    // /// Get the hash of the header excluding the seal
    // pub fn bare_hash(&self) -> H256 {
    //     let mut hash = self.bare_hash.borrow_mut();
    //     // match &mut *hash {
    //     //     &mut Some(ref h) => h.clone(),
    //     //     // hash @ &mut None => {
    //     //     //     let h = self.rlp_keccak(Seal::Without);
    //     //     //     *hash = Some(h.clone());
    //     //     //     h
    //     //     // }
    //     // }
    //     H256::default()
    // }

    /// Note that some fields have changed. Resets the memoised hash.
    pub fn note_dirty(&self) {
        //*self.hash = Some(H256::default());
        *self.hash.borrow_mut() = None;
    }

    // TODO: make these functions traity
    /// Place this header into an RLP stream `s`, optionally `with_seal`.
    pub fn stream_rlp(&self, s: &mut RlpStream, with_seal: Seal) {
        println!("IS LIST(Encodable): {:?}", self.extra_data.to_vec());

        // s.begin_list(10);
        // // parent_hash
        // s.append(&self.parent_hash.as_ref());
        // // coinbase
        // s.append(&self.coinbase.as_ref());
        // // root
        // s.append(&self.state_root.as_ref());
        // // tx_hash
        // s.append(&self.transactions_root.as_ref());
        // // receipt_hash
        // s.append(&self.receipts_root.as_ref());
        // // bloom
        // s.append(&self.logs_bloom.as_ref());
        // // number
        // s.append(&self.number);
        // // gas_used
        // s.append(&self.gas_used);
        // // time
        // s.append(&self.timestamp);
        // // extra
        // s.append(&self.extra_data);
        // s.append(&self.nonce);

        // ///
        s.begin_list(15);
        s.append(&self.parent_hash);
        s.append(&self.uncle_hash);
        s.append(&self.coinbase);
        s.append(&self.state_root);
        s.append(&self.transactions_root);
        s.append(&self.receipts_root);
        s.append(&self.logs_bloom);
        s.append(&self.difficulty);
        s.append(&self.number);
        s.append(&self.gas_limit);
        s.append(&self.gas_used);
        s.append(&self.timestamp);
        s.append(&self.extra_data);
        s.append(&self.mix_hash);
        s.append(&self.nonce);
    }

    /// Get the RLP of this header, optionally `with_seal`.
    pub fn rlp(&self, with_seal: Seal) -> Bytes {
        let mut s = RlpStream::new();
        self.stream_rlp(&mut s, with_seal);
        s.out().into()
    }

    /// Get the SHA3 (Keccak) of this header, optionally `with_seal`.
    pub fn rlp_keccak(&self, with_seal: Seal) -> H256 {
        //   keccak(self.rlp(with_seal))
        keccak("1")
    }
}

impl Encodable for Header {
    fn rlp_append(&self, s: &mut RlpStream) {
        s.begin_list(15);
        s.append(&self.parent_hash);
        s.append(&self.uncle_hash);
        s.append(&self.coinbase);
        s.append(&self.state_root);
        s.append(&self.transactions_root);
        s.append(&self.receipts_root);
        s.append(&self.logs_bloom);
        s.append(&self.difficulty);
        s.append(&self.number);
        s.append(&self.gas_limit);
        s.append(&self.gas_used);
        s.append(&self.timestamp);
        s.append(&self.extra_data);
        s.append(&self.nonce);
        s.append(&self.mix_hash);
    }
}

impl Decodable for Header {
    fn decode(r: &rlp::Rlp) -> Result<Self, DecoderError> {
        Ok(Header {
            parent_hash: r.val_at(0)?,
            uncle_hash: r.val_at(1)?,
            coinbase: r.val_at(2)?,
            state_root: r.val_at(3)?,
            transactions_root: r.val_at(4)?,
            receipts_root: r.val_at(5)?,
            logs_bloom: r.val_at(6)?,
            difficulty: r.val_at(7)?,
            number: r.val_at(8)?,
            gas_limit: r.val_at(9)?,
            gas_used: r.val_at(10)?,
            timestamp: cmp::min(r.val_at::<U256>(11)?, u64::max_value().into())
                .as_u64()
                .into(),
            extra_data: r.val_at(12)?,
            nonce: r.val_at(13)?,
            mix_hash: r.val_at(14)?,
            hash: RefCell::new(Some(keccak(r.as_raw()))),
        })
    }
}

pub(crate) fn generate_random_header(height: &u64) -> (Header, H256) {
    let header = Header {
        parent_hash: H256::random(),
        coinbase: H160::random(),
        state_root: H256::random(),
        transactions_root: H256::random(),
        receipts_root: H256::random(),
        logs_bloom: Bloom::zero(),
        difficulty: U256::from_dec_str("1").unwrap(),
        number: (*height).into(),
        gas_limit: U256::from_dec_str("1").unwrap(),
        gas_used: U256::from_dec_str("1").unwrap(),
        timestamp: 1.into(),
        extra_data: vec![],
        uncle_hash: H256::default(),
        nonce: 1.into(),
        mix_hash: H256::default(),
        hash: RefCell::new(Some(H256::default())),
    }
    .clone();
    let header_clone = header.clone();

    (header, header_clone.hash())
}

pub(crate) fn generate_random_header_based_on_prev_block(
    height: &u64,
    init_parrent_hash: H256,
) -> Header {
    Header {
        parent_hash: init_parrent_hash,
        uncle_hash: H256::random(),
        coinbase: H160::random(),
        state_root: H256::random(),
        transactions_root: H256::random(),
        receipts_root: H256::random(),
        logs_bloom: Bloom::zero(),
        difficulty: U256::from_dec_str("1").unwrap(),
        number: (*height).into(),
        gas_limit: U256::from_dec_str("1").unwrap(),
        gas_used: U256::from_dec_str("1").unwrap(),
        timestamp: 2.into(),
        extra_data: vec![],
        nonce: 2.into(),
        mix_hash: H256::default(),
        hash: RefCell::new(Some(H256::default())),
    }
}

#[cfg(test)]
mod tests {
    use super::Header;
    use crate::eth_types::header::generate_random_header;
    use ethereum_types::{Address, H256};
    use rlp;

    #[test]
    fn decode_encode_header() {
        let (header, _): (Header, H256) = generate_random_header(&123120);

        let encoded_header = rlp::encode(&header);
        let header_to_compare = rlp::decode::<Header>(&encoded_header).unwrap();

        assert_eq!(header.hash(), header_to_compare.hash());
    }
}
