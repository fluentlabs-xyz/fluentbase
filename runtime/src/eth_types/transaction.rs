use ethereum_types::{Address, H160, H256, U256};
// use ethjson;
// use ethkey::{public_to_address, recover, Public, Secret, Signature};
// use evm::Schedule;
// use heapsize::HeapSizeOf;
use rlp::{self, DecoderError, Encodable, RlpStream};
use std::ops::Deref;
use tiny_keccak::{Hasher, Sha3};

/// Wrapper structure around hex-encoded data.
#[derive(Debug, PartialEq, Eq, Default, Hash, Clone)]
pub struct HexEncode<T>(pub T);

// use rlp::*;

type Bytes = Vec<u8>;
type BlockNumber = u64;

/// Fake address for unsigned transactions as defined by EIP-86.
pub const UNSIGNED_SENDER: Address = H160([0xff; 20]);

/// System sender address for internal state updates.
pub const SYSTEM_ADDRESS: Address = H160([
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xfe,
]);

// /// Transaction action type.
// #[derive(Debug, Clone, PartialEq, Eq)]
// pub enum Action {
//     /// Create creates new contract.
//     Create,
//     /// Calls contract at given address.
//     /// In the case of a transfer, this is the receiver's address.'
//     Call(Address),
// }

// impl Default for Action {
//     fn default() -> Action {
//         Action::Create
//     }
// }

// impl rlp::Decodable for Action {
//     fn decode(rlp: &UntrustedRlp) -> Result<Self, DecoderError> {
//         if rlp.is_empty() {
//             Ok(Action::Create)
//         } else {
//             Ok(Action::Call(rlp.as_val()?))
//         }
//     }
// }

// impl rlp::Encodable for Action {
//     fn rlp_append(&self, s: &mut RlpStream) {
//         match *self {
//             Action::Create => s.append_internal(&""),
//             Action::Call(ref addr) => s.append_internal(addr),
//         };
//     }
// }

/// Transaction activation condition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Condition {
    /// Valid at this block number or later.
    Number(BlockNumber),
    /// Valid at this unix time or later.
    Timestamp(u64),
}

/// A set of information describing an externally-originating message call
/// or contract creation operation.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
    /// Nonce.
    pub nonce: U256,
    /// Gas price.
    pub gas_price: U256,
    /// Gas paid up front for transaction execution.
    pub gas: U256,
    /// Action, can be either call or contract create.
    // pub action: Action,
    /// Transfered value.
    pub value: U256,
    /// Transaction data.
    pub data: Bytes,
}

impl Transaction {
    /// Append object with a without signature into RLP stream
    pub fn rlp_append_unsigned_transaction(&self, s: &mut RlpStream, chain_id: Option<u64>) {
        s.begin_list(if chain_id.is_none() { 6 } else { 9 });
        s.append(&self.nonce);
        s.append(&self.gas_price);
        s.append(&self.gas);
        // s.append(&self.action);
        s.append(&self.value);
        s.append(&self.data);
        if let Some(n) = chain_id {
            s.append(&n);
            s.append(&0u8);
            s.append(&0u8);
        }
    }

    // pub fn hash(&self, chain_id: Option<u64>) -> H256 {
    //     let mut stream = RlpStream::new();
    //     self.rlp_append_unsigned_transaction(&mut stream, chain_id);
    //     // keccak(stream.as_raw())
    // }
}

// impl HeapSizeOf for Transaction {
//     fn heap_size_of_children(&self) -> usize {
//         self.data.heap_size_of_children()
//     }
// }

// impl From<ethjson::state::Transaction> for SignedTransaction {
//     fn from(t: ethjson::state::Transaction) -> Self {
//         let to: Option<ethjson::hash::Address> = t.to.into();
//         let secret = t.secret.map(|s| Secret::from_slice(&s.0));
//         let tx = Transaction {
//             nonce: t.nonce.into(),
//             gas_price: t.gas_price.into(),
//             gas: t.gas_limit.into(),
//             action: match to {
//                 Some(to) => Action::Call(to.into()),
//                 None => Action::Create,
//             },
//             value: t.value.into(),
//             data: t.data.into(),
//         };
//         match secret {
//             Some(s) => tx.sign(&s, None),
//             None => tx.null_sign(1),
//         }
//     }
// }

// impl From<ethjson::transaction::Transaction> for UnverifiedTransaction {
//     fn from(t: ethjson::transaction::Transaction) -> Self {
//         let to: Option<ethjson::hash::Address> = t.to.into();
//         UnverifiedTransaction {
//             unsigned: Transaction {
//                 nonce: t.nonce.into(),
//                 gas_price: t.gas_price.into(),
//                 gas: t.gas_limit.into(),
//                 action: match to {
//                     Some(to) => Action::Call(to.into()),
//                     None => Action::Create,
//                 },
//                 value: t.value.into(),
//                 data: t.data.into(),
//             },
//             r: t.r.into(),
//             s: t.s.into(),
//             v: t.v.into(),
//             hash: 0.into(),
//         }
//         .compute_hash()
//     }
// }

/// Signed transaction information without verified signature.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct UnverifiedTransaction {
    /// Plain Transaction.
    unsigned: Transaction,
    /// The V field of the signature; the LS bit described which half of the curve our point falls
    /// in. The MS bits describe which chain this transaction is for. If 27/28, its for all chains.
    v: u64,
    /// The R field of the signature; helps describe the point on the curve.
    r: U256,
    /// The S field of the signature; helps describe the point on the curve.
    s: U256,
    /// Hash of the transaction
    hash: H256,
}

impl Deref for UnverifiedTransaction {
    type Target = Transaction;

    fn deref(&self) -> &Self::Target {
        &self.unsigned
    }
}

impl rlp::Decodable for UnverifiedTransaction {
    fn decode(d: &rlp::Rlp) -> Result<Self, DecoderError> {
        if d.item_count()? != 9 {
            return Err(DecoderError::RlpIncorrectListLen);
        }

        // @TODO
        let hash = H256::zero();
        Ok(UnverifiedTransaction {
            unsigned: Transaction {
                nonce: d.val_at(0)?,
                gas_price: d.val_at(1)?,
                gas: d.val_at(2)?,
                //action: d.val_at(3)?,
                value: d.val_at(4)?,
                data: d.val_at(5)?,
            },
            v: d.val_at(6)?,
            r: d.val_at(7)?,
            s: d.val_at(8)?,
            hash,
        })
    }
}

impl rlp::Encodable for UnverifiedTransaction {
    fn rlp_append(&self, s: &mut RlpStream) {
        self.rlp_append_sealed_transaction(s)
    }
}

impl UnverifiedTransaction {
    // /// Used to compute hash of created transactions
    // fn compute_hash(mut self) -> UnverifiedTransaction {
    //     let hash = tiny_keccak::keccakf(&*self.rlp_bytes());
    //     self.hash = hash;
    //     self
    // }

    /// Checks is signature is empty.
    pub fn is_unsigned(&self) -> bool {
        self.r.is_zero() && self.s.is_zero()
    }

    /// Append object with a signature into RLP stream
    fn rlp_append_sealed_transaction(&self, s: &mut RlpStream) {
        s.begin_list(9);
        s.append(&self.nonce);
        s.append(&self.gas_price);
        s.append(&self.gas);
        // s.append(&self.action);
        s.append(&self.value);
        s.append(&self.data);
        s.append(&self.v);
        s.append(&self.r);
        s.append(&self.s);
    }

    ///	Reference to unsigned part of this transaction.
    pub fn as_unsigned(&self) -> &Transaction {
        &self.unsigned
    }

    /// 0 if `v` would have been 27 under "Electrum" notation, 1 if 28 or 4 if invalid.
    pub fn standard_v(&self) -> u8 {
        match self.v {
            v if v == 27 || v == 28 || v > 36 => ((v - 1) % 2) as u8,
            _ => 4,
        }
    }

    /// The `v` value that appears in the RLP.
    pub fn original_v(&self) -> u64 {
        self.v
    }

    /// The chain ID, or `None` if this is a global transaction.
    pub fn chain_id(&self) -> Option<u64> {
        match self.v {
            v if self.is_unsigned() => Some(v),
            v if v > 36 => Some((v - 35) / 2),
            _ => None,
        }
    }

    /// Get the hash of this header (keccak of the RLP).
    pub fn hash(&self) -> H256 {
        self.hash
    }

    /// Do basic validation, checking for valid signature and minimum gas,
    // TODO: consider use in block validation.
    #[cfg(feature = "json-tests")]
    pub fn validate(
        self,
        schedule: &Schedule,
        require_low: bool,
        allow_chain_id_of_one: bool,
        allow_empty_signature: bool,
    ) -> Result<UnverifiedTransaction, error::Error> {
        let chain_id = if allow_chain_id_of_one { Some(1) } else { None };
        self.verify_basic(require_low, chain_id, allow_empty_signature)?;
        if !allow_empty_signature || !self.is_unsigned() {
            self.recover_public()?;
        }
        if self.gas < U256::from(self.gas_required(&schedule)) {
            return Err(error::Error::InvalidGasLimit(::unexpected::OutOfBounds {
                min: Some(U256::from(self.gas_required(&schedule))),
                max: None,
                found: self.gas,
            })
            .into());
        }
        Ok(self)
    }
}

/// A `UnverifiedTransaction` with successfully recovered `sender`.
// pub struct SignedTransaction {
//     transaction: UnverifiedTransaction,
//     sender: Address,
//     public: Option<Public>,
// }

// impl HeapSizeOf for SignedTransaction {
//     fn heap_size_of_children(&self) -> usize {
//         self.transaction.unsigned.heap_size_of_children()
//     }
// }

/// Transaction action type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Create creates new contract.
    Create,
    /// Calls contract at given address.
    /// In the case of a transfer, this is the receiver's address.'
    Call(Address),
}

impl Default for Action {
    fn default() -> Action {
        Action::Create
    }
}

impl rlp::Decodable for Action {
    fn decode(r: &rlp::Rlp) -> Result<Self, DecoderError> {
        if r.is_empty() {
            if r.is_data() {
                Ok(Action::Create)
            } else {
                Err(DecoderError::RlpExpectedToBeData)
            }
        } else {
            Ok(Action::Call(r.as_val()?))
        }
    }
}

// impl rlp::Encodable for SignedTransaction {
//     fn rlp_append(&self, s: &mut RlpStream) {
//         self.transaction.rlp_append_sealed_transaction(s)
//     }
// }

// impl Deref for SignedTransaction {
//     type Target = UnverifiedTransaction;
//     fn deref(&self) -> &Self::Target {
//         &self.transaction
//     }
// }

// impl From<SignedTransaction> for UnverifiedTransaction {
//     fn from(tx: SignedTransaction) -> Self {
//         tx.transaction
//     }
// }

// impl SignedTransaction {
//     // /// Try to verify transaction and recover sender.
//     // pub fn new(transaction: UnverifiedTransaction) -> Result<Self, ethkey::Error> {
//     //     if transaction.is_unsigned() {
//     //         Ok(SignedTransaction {
//     //             transaction: transaction,
//     //             sender: UNSIGNED_SENDER,
//     //             public: None,
//     //         })
//     //     } else {
//     //         let public = transaction.recover_public()?;
//     //         let sender = public_to_address(&public);
//     //         Ok(SignedTransaction {
//     //             transaction: transaction,
//     //             sender: sender,
//     //             public: Some(public),
//     //         })
//     //     }
//     // }

//     /// Returns transaction sender.
//     pub fn sender(&self) -> Address {
//         self.sender
//     }

//     /// Checks is signature is empty.
//     pub fn is_unsigned(&self) -> bool {
//         self.transaction.is_unsigned()
//     }
// }

/// Signed Transaction that is a part of canon blockchain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalizedTransaction {
    /// Signed part.
    pub signed: UnverifiedTransaction,
    /// Block number.
    pub block_number: BlockNumber,
    /// Block hash.
    pub block_hash: H256,
    /// Transaction index within block.
    pub transaction_index: usize,
    /// Cached sender
    pub cached_sender: Option<Address>,
}

#[test]
fn test_rlp_l1_msg() {
    let raw_tx_rlp_bytes = hex::decode("7ef901b60b825dc0941a258d17bf244c4df02d40343a7626a9d321e10580b901848ef1332e000000000000000000000000ea08a65b1829af779261e768d609e59279b510f2000000000000000000000000f2ec6b6206f6208e8f9b394efc1a01c1cbde77750000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000b00000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000a4232e87480000000000000000000000002b5ad5c4795c026514f8317c7a215e218dccd6cf0000000000000000000000002b5ad5c4795c026514f8317c7a215e218dccd6cf0000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000008000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000094478cdd110520a8e733e2acf9e543d2c687ea5239")
            .expect("decode tx's hex shall not fail");

    let eth_tx = rlp::decode::<UnverifiedTransaction>(&raw_tx_rlp_bytes).unwrap();
}

#[test]
fn test_rlp_eip1559() {
    // the tx is downloaded from https://etherscan.io/getRawTx?tx=0x1c5bd618bdbc575f71bfe0a54f09bca2997bbf6d90d4f371a509b05e2b3124e3
    let raw_tx_rlp_bytes = hex::decode("02f901e901833c3139842b27f14d86012309ce540083055ca8945f65f7b609678448494de4c87521cdf6cef1e93280b8e4fa558b7100000000000000000000000095ad61b0a150d79219dcf64e1e6cc01f0b64c4ce000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000a0000000000000000000000000000000000000000000000000000000000000000100000000000000000000000016a217dedfacdf9c23edb84b57154f26a15848e60000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000028cad80bb7cf17e27c4c8f893f7945f65f7b609678448494de4c87521cdf6cef1e932e1a0d2dc2a0881b05440a4908cf506b4871b1f7eaa46ea0c5dfdcda5f52bc17164a4f8599495ad61b0a150d79219dcf64e1e6cc01f0b64c4cef842a0ba03decd934aae936605e9d437c401439ec4cefbad5795e0965100f929fe339ca0b36e2afa1a25492257090107ad99d079032e543c8dd1ffcd44cf14a96d3015ac80a0821193127789b107351f670025dd3b862f5836e5155f627a29741a251e8d28e8a07ea1e82b1bf6f29c5d0f1e4024acdb698086ac40c353704d7d5e301fb916f2e3")
        .expect("decode tx's hex shall not fail");

    let eth_tx = rlp::decode::<UnverifiedTransaction>(&raw_tx_rlp_bytes).unwrap();
}

fn test_rlp_pre_eip155() {
    // the tx is downloaded from https://etherscan.io/getRawTx?tx=0x5c504ed432cb51138bcf09aa5e8a410dd4a1e204ef84bfed1be16dfba1b22060
    let raw_tx_rlp_bytes = hex::decode("f86780862d79883d2000825208945df9b87991262f6ba471f09758cde1c0fc1de734827a69801ca088ff6cf0fefd94db46111149ae4bfc179e9b94721fffd821d38d16464b3f71d0a045e0aff800961cfce805daef7016b9b675c137a6a41a548f7b60a3484c06a33a")
        .expect("decode tx's hex shall not fail");

    let eth_tx = rlp::decode::<UnverifiedTransaction>(&raw_tx_rlp_bytes).unwrap();
}
