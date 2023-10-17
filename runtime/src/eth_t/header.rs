pub type Bytes = Vec<u8>;
use crate::hash::{keccak, KECCAK_EMPTY_LIST_RLP, KECCAK_NULL_RLP};
use ethereum_types::{Address, Bloom, H160, H256, U256};
use rlp::*;
use std::{cell::RefCell, cmp};

/// Semantic boolean for when a seal/signature is included.
pub enum Seal {
    /// The seal/signature is included.
    With,
    /// The seal/signature is not included.
    Without,
}

#[derive(Debug, Clone, Eq)]
pub(crate) struct Header {
    /// Parent hash.
    parent_hash: H256,
    /// Block timestamp.
    timestamp: u64,
    /// Block number.
    number: u64,
    /// Block author.
    author: Address,

    /// Transactions root.
    transactions_root: H256,
    /// Block uncles hash.
    uncles_hash: H256,
    /// Block extra data.
    extra_data: Bytes,

    /// State root.
    state_root: H256,
    /// Block receipts root.
    receipts_root: H256,
    /// Block bloom.
    logs_bloom: Bloom,
    /// Gas used for contracts execution.
    gas_used: U256,
    /// Block gas limit.
    gas_limit: U256,

    /// Block difficulty.
    difficulty: U256,
    /// Vector of post-RLP-encoded fields.
    seal: Vec<Bytes>,

    /// The memoized hash of the RLP representation *including* the seal fields.
    hash: RefCell<Option<H256>>,
    /// The memoized hash of the RLP representation *without* the seal fields.
    bare_hash: RefCell<Option<H256>>,
}

impl PartialEq for Header {
    fn eq(&self, c: &Header) -> bool {
        self.parent_hash == c.parent_hash
            && self.timestamp == c.timestamp
            && self.number == c.number
            && self.author == c.author
            && self.transactions_root == c.transactions_root
            && self.uncles_hash == c.uncles_hash
            && self.extra_data == c.extra_data
            && self.state_root == c.state_root
            && self.receipts_root == c.receipts_root
            && self.logs_bloom == c.logs_bloom
            && self.gas_used == c.gas_used
            && self.gas_limit == c.gas_limit
            && self.difficulty == c.difficulty
            && self.seal == c.seal
    }
}

impl Default for Header {
    fn default() -> Self {
        Header {
            parent_hash: H256::default(),
            timestamp: 0,
            number: 0,
            author: Address::default(),

            transactions_root: KECCAK_NULL_RLP,
            uncles_hash: KECCAK_EMPTY_LIST_RLP,
            extra_data: vec![],

            state_root: KECCAK_NULL_RLP,
            receipts_root: KECCAK_NULL_RLP,
            logs_bloom: Bloom::default(),
            gas_used: U256::default(),
            gas_limit: U256::default(),

            difficulty: U256::default(),
            seal: vec![],
            hash: RefCell::new(None),
            bare_hash: RefCell::new(None),
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
    pub fn timestamp(&self) -> u64 {
        self.timestamp
    }
    /// Get the number field of the header.
    pub fn number(&self) -> u64 {
        self.number
    }
    /// Get the author field of the header.
    pub fn author(&self) -> &Address {
        &self.author
    }

    /// Get the extra data field of the header.
    pub fn extra_data(&self) -> &Bytes {
        &self.extra_data
    }
    /// Get a mutable reference to extra_data
    pub fn extra_data_mut(&mut self) -> &mut Bytes {
        self.note_dirty();
        &mut self.extra_data
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
    pub fn uncles_hash(&self) -> &H256 {
        &self.uncles_hash
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
    /// Get the seal field of the header.
    pub fn seal(&self) -> &[Bytes] {
        &self.seal
    }

    /// Get the seal field with RLP-decoded values as bytes.
    pub fn decode_seal<'a, T: ::std::iter::FromIterator<&'a [u8]>>(
        &'a self,
    ) -> Result<T, DecoderError> {
        self.seal.iter().map(|rlp| Rlp::new(rlp).data()).collect()
    }

    /// Set the number field of the header.
    pub fn set_parent_hash(&mut self, a: H256) {
        self.parent_hash = a;
        self.note_dirty();
    }
    /// Set the uncles hash field of the header.
    pub fn set_uncles_hash(&mut self, a: H256) {
        self.uncles_hash = a;
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
    pub fn set_timestamp(&mut self, a: u64) {
        self.timestamp = a;
        self.note_dirty();
    }
    // /// Set the timestamp field of the header to the current time.
    // pub fn set_timestamp_now(&mut self, but_later_than: u64) {
    //     self.timestamp = cmp::max(get_time().sec as u64, but_later_than + 1);
    //     self.note_dirty();
    // }
    /// Set the number field of the header.
    pub fn set_number(&mut self, a: u64) {
        self.number = a;
        self.note_dirty();
    }
    /// Set the author field of the header.
    pub fn set_author(&mut self, a: Address) {
        if a != self.author {
            self.author = a;
            self.note_dirty();
        }
    }

    /// Set the extra data field of the header.
    pub fn set_extra_data(&mut self, a: Bytes) {
        if a != self.extra_data {
            self.extra_data = a;
            self.note_dirty();
        }
    }

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
    /// Set the seal field of the header.
    pub fn set_seal(&mut self, a: Vec<Bytes>) {
        self.seal = a;
        self.note_dirty();
    }

    /// Get the hash of this header (keccak of the RLP).
    pub fn hash(&self) -> H256 {
        let mut hash = self.hash.borrow_mut();
        match &mut *hash {
            &mut Some(ref h) => h.clone(),
            hash @ &mut None => {
                let h = self.rlp_keccak(Seal::With);
                *hash = Some(h.clone());
                h
            }
        }
    }

    /// Get the hash of the header excluding the seal
    pub fn bare_hash(&self) -> H256 {
        let mut hash = self.bare_hash.borrow_mut();
        match &mut *hash {
            &mut Some(ref h) => h.clone(),
            hash @ &mut None => {
                let h = self.rlp_keccak(Seal::Without);
                *hash = Some(h.clone());
                h
            }
        }
    }

    /// Note that some fields have changed. Resets the memoised hash.
    pub fn note_dirty(&self) {
        // *self.hash.borrow_mut() = None;
        *self.bare_hash.borrow_mut() = None;
    }

    // TODO: make these functions traity
    /// Place this header into an RLP stream `s`, optionally `with_seal`.
    pub fn stream_rlp(&self, s: &mut RlpStream, with_seal: Seal) {
        s.begin_list(
            13 + match with_seal {
                Seal::With => self.seal.len(),
                _ => 0,
            },
        );
        s.append(&self.parent_hash);
        s.append(&self.uncles_hash);
        s.append(&self.author);
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
        if let Seal::With = with_seal {
            for b in &self.seal {
                s.append_raw(b, 1);
            }
        }
    }

    /// Get the RLP of this header, optionally `with_seal`.
    pub fn rlp(&self, with_seal: Seal) -> Bytes {
        let mut s = RlpStream::new();
        self.stream_rlp(&mut s, with_seal);
        s.out().to_vec()
    }

    /// Get the SHA3 (Keccak) of this header, optionally `with_seal`.
    pub fn rlp_keccak(&self, with_seal: Seal) -> H256 {
        keccak(self.rlp(with_seal))
    }

    // /// Returns the rlp length of the Header body, _not including_ trailing EIP155 fields or the
    // /// rlp list header
    // /// To get the length including the rlp list header, refer to the Encodable implementation.
    // pub(crate) fn header_payload_length(&self) -> usize {
    //     let mut length = 0;
    //     length += self.parent_hash.as_bytes().len();
    //     length += self.state_root.as_bytes().len();
    //     length += self.transactions_root.as_bytes().len();
    //     length += self.receipts_root.as_bytes().len();
    //     length += self.logs_bloom.as_bytes().len();
    //     length += self.difficulty.as_bytes().len();
    //     length += 64;
    //     length += self.gas_limit.length += self.gas_used.as_bytes().len();
    //     length += self.timestamp.as_bytes().len();
    //     length += self.extra_data.as_mut_slice().len();
    //     length += self.mix_hash.as_bytes().len();
    //     length += self.nonce.as_bytes().len();
    //     length += self
    //         .base_fee_per_gas
    //         .map(|fee| fee.length())
    //         .unwrap_or_default();

    //     length
    // }
}

impl Decodable for Header {
    fn decode(r: &rlp::Rlp) -> Result<Self, DecoderError> {
        let mut blockheader = Header {
            parent_hash: r.val_at(0)?,
            uncles_hash: r.val_at(1)?,
            author: r.val_at(2)?,
            state_root: r.val_at(3)?,
            transactions_root: r.val_at(4)?,
            receipts_root: r.val_at(5)?,
            logs_bloom: r.val_at(6)?,
            difficulty: r.val_at(7)?,
            number: r.val_at(8)?,
            gas_limit: r.val_at(9)?,
            gas_used: r.val_at(10)?,
            timestamp: cmp::min(r.val_at::<U256>(11)?, u64::max_value().into()).as_u64(),
            extra_data: r.val_at(12)?,
            seal: vec![],
            hash: RefCell::new(Some(keccak(r.as_raw()))),
            bare_hash: RefCell::new(None),
        };

        for i in 13..r.item_count()? {
            blockheader.seal.push(r.at(i)?.as_raw().to_vec())
        }

        Ok(blockheader)
    }
}

impl Encodable for Header {
    fn rlp_append(&self, s: &mut RlpStream) {
        self.stream_rlp(s, Seal::With);
    }
}

pub(crate) fn generate_random_header(height: &u64) -> Header {
    Header {
        parent_hash: H256::random(),
        uncles_hash: H256::random(),
        author: H160::random(),
        state_root: H256::random(),
        transactions_root: H256::random(),
        receipts_root: H256::random(),
        logs_bloom: Bloom::zero(),
        difficulty: U256::from_dec_str("1").unwrap(),
        number: *height,
        gas_limit: U256::from_dec_str("1").unwrap(),
        gas_used: U256::from_dec_str("1").unwrap(),
        timestamp: 1,
        extra_data: vec![],
        seal: vec![],
        hash: RefCell::new(None),
        bare_hash: RefCell::new(None),
    }
}

pub(crate) fn generate_random_header_based_on_prev_block(
    height: &u64,
    init_parrent_hash: H256,
) -> Header {
    Header {
        parent_hash: init_parrent_hash,
        uncles_hash: H256::random(),
        author: H160::random(),
        state_root: H256::random(),
        transactions_root: H256::random(),
        receipts_root: H256::random(),
        logs_bloom: Bloom::zero(),
        difficulty: U256::from_dec_str("1").unwrap(),
        number: *height,
        gas_limit: U256::from_dec_str("1").unwrap(),
        gas_used: U256::from_dec_str("1").unwrap(),
        timestamp: 1,
        extra_data: vec![],
        seal: vec![],
        hash: RefCell::new(None),
        bare_hash: RefCell::new(None),
    }
}

#[cfg(test)]
mod tests {
    use super::Header;
    use crate::eth_t::header::generate_random_header;
    use ethereum_types::{Address, H256};
    use rlp;

    #[test]
    fn decode_encode_header() {
        let header = generate_random_header(&123120);

        let encoded_header = rlp::encode(&header);
        let header_to_compare = rlp::decode::<Header>(&encoded_header).unwrap();

        assert_eq!(header.hash(), header_to_compare.hash());
    }
}
