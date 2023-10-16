//use web3;

extern crate web3;

use ethereum_types::{Address, H128, H160, H256, U128, U256};
use rlp::{encode, RlpStream};
use std::str::FromStr;
use web3::{
    contract::Options,
    signing::keccak256,
    types::{Block, Transaction},
};

struct MyWrapperType(Block<Transaction>);

impl rlp::Encodable for MyWrapperType {
    fn rlp_append(&self, s: &mut RlpStream) {
        // let block_hash = b.hash;
        // let transactions = b.transactions;

        s.append(&self.0.hash);
        // s.append_list(&self.0.transactions);
        //s.append_list(&self.0.transactions);
        // // Encode the block data into RLP

        // //  let encoded_data = encode(&b);

        println!("Block Hash: {:?}", self.0.hash);
        // println!("Transactions: {:?}", transactions);
    }
}

// #[cfg(test)]
// mod tests {
//     #[test]
//     fn test_decode_block() {
//         let hex_encoded_rlp =
// "f9025cf90257a0736a0e40f92be952dd3bb1c9bc82a8cceeb8a486d566ffebf6628a845e76dc8ca01dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d4934794b71b214cb885500844365e95cd9942c7276e7fd8a00b9279d6596c22b580a56e87110ab3f78a3dce913ffb7a2b157e2ed7b7146859a056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421a056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421b9010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000020a84024b0e3580845f06ce1db861d983010000846765746889676f312e31322e3137856c696e75780000000000009c6c5eb3627786adce83c21578c77ed89f1048c48c8697b6d21a7cc23b66cb3d1ef61fbb1fb7bbe6ba7b06b2ad5eebca8fac470bfbb0e2cae279e751ec91d48f01a00000000000000000000000000000000000000000000000000000000000000000880000000000000000c0c0"
// ;         let input_data = hex::decode(hex_encoded_rlp).expect("Failed to decode hex string");

//         let transport = web3::transports::Http::new("http://localhost:8545");
//         let web3 = web3::Web3::new(transport);

//         println!("Calling accounts.");
//         let mut accounts = web3.eth().accounts().await?;
//         println!("Accounts: {:?}", accounts);
//         accounts.push("00a329c0648769a73afac7f9381e08fb43dbea72".parse().unwrap());

//         println!("Calling balance.");
//         for account in accounts {
//             let balance = web3.eth().balance(account, None).await?;
//             println!("Balance of {:?}: {}", account, balance);
//         }
//     }
// }

pub(crate) async fn get_block() -> web3::Result<()> {
    let transport = web3::transports::Http::new("")?;
    let web3 = web3::Web3::new(transport);

    println!("Calling accounts.");
    //  let mut accounts = web3.eth().accounts().await?;
    let blk = web3.eth().block_number().await?;

    println!("Accounts: {:?}", blk);
    let hex_string = "0xf7af83cdf85e807101d098649234cc2d034b0aacf42e7359129d656b74a067b8";
    let block_hash = H256::from_str(hex_string).expect("Failed to parse hex string");
    let block: Option<Block<Transaction>> = web3
        .eth()
        .block_with_txs(web3::types::BlockId::Hash(block_hash))
        .await?;
    if block.is_none() {
        panic!("Block not found!!!")
    }
    let block = block.unwrap();
    println!("block: {:?}", block);

    let custom_block = MyWrapperType(block);
    let encoded_data = encode(&custom_block);

    println!("{:?}", encoded_data);

    // accounts.push("00a329c0648769a73afac7f9381e08fb43dbea72".parse().unwrap());

    // println!("Calling balance.");
    // for account in accounts {
    //     let balance = web3.eth().balance(account, None).await?;
    //     println!("Balance of {:?}: {}", account, balance);
    // }

    Ok(())
}

// use rlp::{self, Encodable, RlpStream};
// use std::fmt::{self, Display, Formatter};
// use tiny_keccak::Keccak;
// use web3::types::{Address, Bytes, H160, H2048, H256, U128, U256};

/// A block represents a block of a blockchain.
// #[derive(Debug)]
pub struct BlockX {
    /// The block hash of this block.
    pub hash: H256,
    pub parent_hash: H256,
    pub uncles_hash: H256,
    pub author: H160,
    pub state_root: H256,
    pub transactions_root: H256,
    pub receipts_root: H256,
    //pub logs_bloom: H2048,
    pub total_difficulty: U256,
    pub number: U128,
    pub gas_limit: U256,
    pub gas_used: U256,
    pub timestamp: U256,
    //    pub extra_data: Bytes,
    pub mix_data: H256,
    pub nonce: U256,
    pub events: Vec<Event>,
}

// impl Display for Block {
//     fn fmt(&self, fmt: &mut Formatter) -> Result<(), fmt::Error> {
//         write!(fmt, "Block ({:x})", self.hash);

//         Ok(())
//     }
// }

// impl Block {
//     /// Calculate hash of block.
//     pub fn hash(&self) -> H256 {
//         let encoded_block = rlp::encode(self);
//         let mut res: [u8; 32] = [0; 32];
//         //  keccak256(encoded_block.as_slice(), &mut res);
//         H256::from(res)
//     }
// }

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub address: Address,
    pub topics: Vec<H256>,
    // pub data: Bytes,
    pub block_hash: Option<H256>,
    pub block_number: Option<U256>,
    pub transaction_hash: Option<H256>,
    pub transaction_index: Option<U256>,
    pub log_index: Option<U256>,
    pub transaction_log_index: Option<U256>,
    pub log_type: Option<String>,
    pub removed: Option<bool>,
}

pub struct BlockH256(pub Option<Block<H256>>);

impl rlp::Encodable for BlockH256 {
    fn rlp_append(&self, s: &mut RlpStream) {
        match &self.0 {
            Some(block) => {
                s.append(&block.author);
                // s.append(block.state_root.rlp_bytes());
                // xxx = block.try_into().expect("");
            }
            None => {
                // Encode a placeholder or handle the case when the Block<H256> is None
                // For example, you could encode a default value or leave it empty
                // s.begin_list(0);
                // s.finalize();
            }
        }
    }
}

// impl rlp::Encodable for Block<H256>  {
//     /// RLP encode block
//     ///
//     /// # Arguments
//     ///
//     /// * `rlp_stream` - Appendable rlp encoder.
//     fn rlp_append(&self, rlp_stream: &mut rlp::RlpStream) {
//         rlp_stream.begin_list(15);
//         let Options<Block {
//             hash,
//             parent_hash,
//             uncles_hash,
//             author,
//             state_root,
//             transactions_root,
//             receipts_root,
//             number,
//             gas_used,
//             gas_limit,
//             base_fee_per_gas,
//             extra_data,
//             logs_bloom,
//             timestamp,
//             difficulty,
//             total_difficulty,
//             seal_fields,
//             uncles,
//             transactions,
//             size,
//             mix_hash,
//             nonce,
//         } = &self.0;

//         rlp_stream.append(parent_hash);
//         rlp_stream.append(uncles_hash);
//         rlp_stream.append(author);
//         rlp_stream.append(state_root);
//         rlp_stream.append(transactions_root);
//         rlp_stream.append(receipts_root);
//         rlp_stream.append(logs_bloom);
//         rlp_stream.append(total_difficulty);
//         rlp_stream.append(number);
//         rlp_stream.append(gas_limit);
//         rlp_stream.append(gas_used);
//         rlp_stream.append(timestamp);
//         rlp_stream.append(hash);
//         rlp_stream.append(mix_hash);
//         rlp_stream.append(nonce);
//     }
// }

pub(crate) async fn decode_block_with_txs() -> web3::Result<()> {
    let transport = web3::transports::Http::new("")?;
    let web3 = web3::Web3::new(transport);

    // //  let mut accounts = web3.eth().accounts().await?;
    // let blk = web3.eth().block_number().await?;

    let hex_string = "0xf7af83cdf85e807101d098649234cc2d034b0aacf42e7359129d656b74a067b8";
    let block_hash = H256::from_str(hex_string).expect("Failed to parse hex string");
    let block = web3
        .eth()
        .block(web3::types::BlockId::Hash(block_hash))
        .await?;
    // println!("block: {:?}", block);

    let block_x = BlockH256(block);

    let rlp_encoded_data = encode(&block_x).to_vec();

    println!("!!!!! {:?}", rlp_encoded_data);

    // accounts.push("00a329c0648769a73afac7f9381e08fb43dbea72".parse().unwrap());

    // println!("Calling balance.");
    // for account in accounts {
    //     let balance = web3.eth().balance(account, None).await?;
    //     println!("Balance of {:?}: {}", account, balance);
    // }

    Ok(())
}
