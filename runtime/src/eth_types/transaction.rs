use crate::eth_types::tx_types::TxType::PreEip155;
use ethereum_types::{Address, H160, H256, U256};
use ethers_core::{
    k256::ecdsa::SigningKey,
    types::{
        transaction::eip2718::TypedTransaction,
        Bytes,
        Eip1559TransactionRequest,
        Eip2930TransactionRequest,
        NameOrAddress,
        Signature,
        TransactionRequest,
    },
};
use keccak_hash::keccak;
use rlp::{self, DecoderError, Encodable, RlpStream};
pub type Word = U256;

use super::{gas_utils::tx_data_gas_cost, tx_types::TxType};
use std::{cmp::Ordering, collections::BTreeMap};

/// Transaction in a witness block
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Transaction {
    /// The block number in which this tx is included in
    pub block_number: u64,
    /// The transaction identifier in the block
    pub id: usize,
    /// The hash of the transaction
    pub hash: H256,
    /// The type of the transaction
    pub tx_type: TxType,
    /// The sender account nonce of the transaction
    pub nonce: u64,
    /// The gas limit of the transaction
    pub gas: u64,
    /// The gas price
    pub gas_price: Word,
    /// The caller address
    pub caller_address: Address,
    /// The callee address
    pub callee_address: Option<Address>,
    /// Whether it's a create transaction
    pub is_create: bool,
    /// The ether amount of the transaction
    pub value: Word,
    /// The call data
    pub call_data: Vec<u8>,
    /// The call data length
    pub call_data_length: usize,
    /// The gas cost for transaction call data
    pub call_data_gas_cost: u64,
    /// The gas cost for rlp-encoded bytes of unsigned tx
    pub tx_data_gas_cost: u64,
    /// Chain ID as per EIP-155.
    pub chain_id: u64,
    /// Rlp-encoded bytes of unsigned tx
    pub rlp_unsigned: Vec<u8>,
    /// Rlp-encoded bytes of signed tx
    pub rlp_signed: Vec<u8>,
    /// "v" value of the transaction signature
    pub v: u64,
    /// "r" value of the transaction signature
    pub r: Word,
    /// "s" value of the transaction signature
    pub s: Word,
    //     / The calls made in the transaction
    //     / @TODO
    //     pub calls: Vec<Call>,
    //     / The steps executioned in the transaction
    //   /  pub steps: Vec<ExecStep>,
}

// impl From<&Transaction> for TransactionRequest {
//     fn from(tx: &Transaction) -> TransactionRequest {
//         TransactionRequest {
//             from: Some(tx.from),
//             to: tx.to.map(NameOrAddress::Address),
//             gas: Some(tx.gas_limit),
//             gas_price: Some(tx.gas_price),
//             value: Some(tx.value),
//             data: Some(tx.call_data.clone()),
//             nonce: Some(tx.nonce),
//             ..Default::default()
//         }
//     }
// }

/// Generate a dummy pre-eip155 tx in which
/// (nonce=0, gas=0, gas_price=0, to=0, value=0, data="")
/// using the dummy private key = 1
pub fn get_dummy_tx_request() -> (TransactionRequest) {
    let mut sk_be_scalar = [0u8; 32];
    sk_be_scalar[31] = 1_u8;

    let sk = SigningKey::from_bytes((&sk_be_scalar).into()).expect("sign key = 1");
    // let wallet = ethers_signers::Wallet::from(sk);

    let tx = TransactionRequest::new()
        .nonce(0)
        .gas(0)
        .gas_price(U256::zero())
        .to(Address::zero())
        .value(U256::zero())
        .data(Bytes::default());
    let sighash: H256 = keccak(tx.rlp_unsigned()).into();

    // TODO add signing

    tx
}

/// Get the tx hash of the dummy tx (nonce=0, gas=0, gas_price=0, to=0, value=0,
/// data="")
pub fn compute_dummy_tx_hash(tx_request: &TransactionRequest) -> H256 {
    keccak(tx_request.rlp_unsigned())
}

impl Transaction {
    /// Return a fixed dummy pre-eip155 tx
    pub fn dummy(chain_id: u64) -> Self {
        let dummy_tx_request = get_dummy_tx_request();
        let dummy_tx_request_hash = compute_dummy_tx_hash(&dummy_tx_request);
        //  let rlp_signed = dummy_tx.rlp_signed(&dummy_sig).to_vec();
        let rlp_unsigned = dummy_tx_request.rlp_unsigned().to_vec();

        Self {
            block_number: 0,
            id: 0, // need to be changed to correct value
            caller_address: Address::zero(),
            callee_address: Some(Address::zero()),
            is_create: false, // callee_address != None
            chain_id,
            tx_data_gas_cost: 0, // TODO
            // v: dummy_sig.v,
            // r: dummy_sig.r,
            // s: dummy_sig.s,
            // rlp_signed,
            rlp_unsigned,
            hash: dummy_tx_request_hash,
            tx_type: PreEip155,

            ..Default::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn new_from_rlp_bytes(
        tx_type: TxType,
        signed_bytes: Vec<u8>,
        unsigned_bytes: Vec<u8>,
    ) -> Self {
        Self {
            id: 1,
            tx_type,
            rlp_signed: signed_bytes,
            rlp_unsigned: unsigned_bytes,
            ..Default::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn new_from_rlp_signed_bytes(tx_type: TxType, bytes: Vec<u8>) -> Self {
        Self {
            id: 1,
            tx_type,
            rlp_signed: bytes,
            ..Default::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn new_from_rlp_unsigned_bytes(tx_type: TxType, bytes: Vec<u8>) -> Self {
        Self {
            id: 1,
            tx_type,
            rlp_unsigned: bytes,
            ..Default::default()
        }
    }
}

impl rlp::Encodable for Transaction {
    fn rlp_append(&self, s: &mut RlpStream) {
        self.rlp_append(s)
    }
}

impl rlp::Decodable for Transaction {
    fn decode(d: &rlp::Rlp) -> Result<Self, DecoderError> {
        if d.item_count()? != 9 {
            return Err(DecoderError::RlpIncorrectListLen);
        }

        let hash = H256::zero();
        Ok(Transaction {
            nonce: d.val_at(0)?,
            gas_price: d.val_at(1)?,
            gas: d.val_at(2)?,
            //action: d.val_at(3)?,
            value: d.val_at(4)?,

            v: d.val_at(6)?,
            r: d.val_at(7)?,
            s: d.val_at(8)?,
            hash,
            block_number: todo!(),
            id: todo!(),
            tx_type: todo!(),
            caller_address: todo!(),
            callee_address: todo!(),
            is_create: todo!(),
            call_data: todo!(),
            call_data_length: todo!(),
            call_data_gas_cost: todo!(),
            tx_data_gas_cost: todo!(),
            chain_id: todo!(),
            rlp_unsigned: todo!(),
            rlp_signed: todo!(),
        })
    }
}

// impl From<MockTransaction> for Transaction {
//     fn from(mock_tx: MockTransaction) -> Self {
//         let is_create = mock_tx.to.is_none();
//         let sig = Signature {
//             r: mock_tx.r.expect("tx expected to be signed"),
//             s: mock_tx.s.expect("tx expected to be signed"),
//             v: mock_tx.v.expect("tx expected to be signed").as_u64(),
//         };
//         let (rlp_unsigned, rlp_signed) = {
//             let mut legacy_tx = TransactionRequest::new()
//                 .from(mock_tx.from.address())
//                 .nonce(mock_tx.nonce)
//                 .gas_price(mock_tx.gas_price)
//                 .gas(mock_tx.gas)
//                 .value(mock_tx.value)
//                 .data(mock_tx.input.clone())
//                 .chain_id(mock_tx.chain_id);
//             if !is_create {
//                 legacy_tx = legacy_tx.to(mock_tx.to.as_ref().map(|to| to.address()).unwrap());
//             }

//             let unsigned = legacy_tx.rlp().to_vec();
//             let signed = legacy_tx.rlp_signed(&sig).to_vec();

//             (unsigned, signed)
//         };
//         Self {
//             block_number: 1,
//             id: mock_tx.transaction_index.as_usize(),
//             hash: mock_tx.hash.unwrap_or_default(),
//             tx_type: TxType::Eip155,
//             nonce: mock_tx.nonce.as_u64(),
//             gas: mock_tx.gas.as_u64(),
//             gas_price: mock_tx.gas_price,
//             caller_address: mock_tx.from.address(),
//             callee_address: mock_tx.to.as_ref().map(|to| to.address()),
//             is_create,
//             value: mock_tx.value,
//             call_data: mock_tx.input.to_vec(),
//             call_data_length: mock_tx.input.len(),
//             call_data_gas_cost: tx_data_gas_cost(&mock_tx.input),
//             tx_data_gas_cost: tx_data_gas_cost(&rlp_signed),
//             chain_id: mock_tx.chain_id,
//             rlp_unsigned,
//             rlp_signed,
//             v: sig.v,
//             r: sig.r,
//             s: sig.s,
//             l1_fee: Default::default(),
//             l1_fee_committed: Default::default(),
//             calls: vec![],
//             steps: vec![],
//         }
//     }
// }

#[cfg(test)]
mod tests {
    use crate::eth_types::transaction::Transaction;
    use ethers_core::{
        types::{Transaction as EthTransaction, TransactionRequest},
        utils::rlp::{Decodable, Rlp},
    };
    use fluentbase_rwasm::Value;

    #[test]
    fn test_rlp_pre_eip155() {
        // the tx is downloaded from https://etherscan.io/getRawTx?tx=0x5c504ed432cb51138bcf09aa5e8a410dd4a1e204ef84bfed1be16dfba1b22060
        let raw_tx_rlp_bytes = hex::decode("f86780862d79883d2000825208945df9b87991262f6ba471f09758cde1c0fc1de734827a69801ca088ff6cf0fefd94db46111149ae4bfc179e9b94721fffd821d38d16464b3f71d0a045e0aff800961cfce805daef7016b9b675c137a6a41a548f7b60a3484c06a33a")
            .expect("decode tx's hex shall not fail");

        let eth_tx = EthTransaction::decode(&Rlp::new(&raw_tx_rlp_bytes))
            .expect("decode tx's rlp bytes shall not fail");

        println!("{:?}", eth_tx)
    }

    #[test]
    fn test_rlp_eip1559() {
        // the tx is downloaded from https://etherscan.io/getRawTx?tx=0x1c5bd618bdbc575f71bfe0a54f09bca2997bbf6d90d4f371a509b05e2b3124e3
        let raw_tx_rlp_bytes = hex::decode("02f901e901833c3139842b27f14d86012309ce540083055ca8945f65f7b609678448494de4c87521cdf6cef1e93280b8e4fa558b7100000000000000000000000095ad61b0a150d79219dcf64e1e6cc01f0b64c4ce000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000a0000000000000000000000000000000000000000000000000000000000000000100000000000000000000000016a217dedfacdf9c23edb84b57154f26a15848e60000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000028cad80bb7cf17e27c4c8f893f7945f65f7b609678448494de4c87521cdf6cef1e932e1a0d2dc2a0881b05440a4908cf506b4871b1f7eaa46ea0c5dfdcda5f52bc17164a4f8599495ad61b0a150d79219dcf64e1e6cc01f0b64c4cef842a0ba03decd934aae936605e9d437c401439ec4cefbad5795e0965100f929fe339ca0b36e2afa1a25492257090107ad99d079032e543c8dd1ffcd44cf14a96d3015ac80a0821193127789b107351f670025dd3b862f5836e5155f627a29741a251e8d28e8a07ea1e82b1bf6f29c5d0f1e4024acdb698086ac40c353704d7d5e301fb916f2e3")
            .expect("decode tx's hex shall not fail");

        let eth_tx = EthTransaction::decode(&Rlp::new(&raw_tx_rlp_bytes))
            .expect("decode tx's rlp bytes shall not fail");

        println!("{:?}", eth_tx)
    }
}
