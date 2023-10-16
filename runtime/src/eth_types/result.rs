// // use ethereum::{Block, TransactionAny};
// use codec::{Decode, Encode};
// use ethereum_types::{Address, H128, H160, H256, U128, U256};
// use reqwest::Error;
// use serde_json::*;

// /// An Ethereum block header.
// #[derive(Encode, Decode)]
// #[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
// pub struct Header {
//     /// Parent block hash.
//     pub parent_hash: H256,
//     /// Block timestamp.
//     pub timestamp: u64,
//     /// Block number.
//     pub number: u64,
//     /// Block author.
//     pub author: Address,

//     /// Transactions root.
//     pub transactions_root: H256,
//     /// Block ommers hash.
//     pub ommers_hash: H256,
//     /// Block extra data.
//     // pub extra_data: Bytes,

//     /// State root.
//     pub state_root: H256,
//     /// Block receipts root.
//     pub receipts_root: H256,
//     /// Block bloom.
//     //pub logs_bloom: Bloom,
//     /// Gas used for contracts execution.
//     pub gas_used: U256,
//     /// Block gas limit.
//     pub gas_limit: U256,

//     /// Block difficulty.
//     pub difficulty: U256,
//     /// Vector of post-RLP-encoded fields.
//     // pub seal: Vec<Bytes>,

//     // Base fee per gas (EIP-1559), only in headers from the London hardfork onwards.
//     pub base_fee: Option<U256>,
// }

// // #[proc_macro_attribute]
// pub fn derive_arbitrary(args: TokenStream, input: TokenStream) -> TokenStream {
//     let ast = parse_macro_input!(input as DeriveInput);

//     let tests = arbitrary::maybe_generate_tests(args, &ast);

//     // Avoid duplicate names
//     let prop_import = format_ident!("{}PropTestArbitratry", ast.ident);
//     let arb_import = format_ident!("{}Arbitratry", ast.ident);

//     quote! {
//         #[cfg(any(test, feature = "arbitrary"))]
//         use proptest_derive::Arbitrary as #prop_import;

//         #[cfg(any(test, feature = "arbitrary"))]
//         use arbitrary::Arbitrary as #arb_import;

//         #[cfg_attr(any(test, feature = "arbitrary"), derive(#prop_import, #arb_import))]
//         #ast

//         #tests
//     }
//     .into()
// }

// /// Ethereum full block.
// ///
// /// Withdrawals can be optionally included at the end of the RLP encoded message.
// #[derive(
//     Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, RlpEncodable, RlpDecodable,
// )]
// #[derive_arbitrary(rlp, 25)]
// #[rlp(trailing)]
// pub struct Block {
//     /// Block header.
//     pub header: Header,
//     /// Transactions in this block.
//     pub body: Vec<TransactionSigned>,
//     /// Ommers/uncles header.
//     pub ommers: Vec<Header>,
//     /// Block withdrawals.
//     pub withdrawals: Option<Vec<Withdrawal>>,
// }

// async fn get_block() {
//     // Specify the Ethereum node URL
//     let node_url = "http://localhost:8545"; // Replace with your Ethereum node URL

//     // Specify the block number (in hex format)
//     let block_number = "0x2"; // Replace with the block number you're interested in

//     // Build the request URL
//     let url = format!("{}/{}", node_url, "geth_getBlockByNumber");

//     // Create the request JSON body
//     let request_body = json!([block_number, true]);

//     // Send the request
//     let response = reqwest::Client::new()
//         .post(&url)
//         .json(&request_body)
//         .send()
//         .await
//         .expect("");

//     // Deserialize the JSON response into EthereumBlock struct

//     // let block: Block = serde_json::from_value(&response.json()).expect("");
// }

// #[test]
// fn encode_decode_raw_block() {
//     let bytes =
// hex!("f90288f90218a0fe21bb173f43067a9f90cfc59bbb6830a7a2929b5de4a61f372a9db28e87f9aea01dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347940000000000000000000000000000000000000000a061effbbcca94f0d3e02e5bd22e986ad57142acabf0cb3d129a6ad8d0f8752e94a0d911c25e97e27898680d242b7780b6faef30995c355a2d5de92e6b9a7212ad3aa0056b23fbba480696b65fe5a59b8f2148a1299103c4f57df839233af2cf4ca2d2b90100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000008003834c4b408252081e80a00000000000000000000000000000000000000000000000000000000000000000880000000000000000842806be9da056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421f869f86702842806be9e82520894658bdf435d810c91414ec09147daa6db624063798203e880820a95a040ce7918eeb045ebf8c8b1887ca139d076bda00fa828a07881d442a72626c42da0156576a68e456e295e4c9cf67cf9f53151f329438916e0f24fc69d6bbb7fbacfc0c0"
// );     let bytes_buf = &mut bytes.as_ref();
//     let mut encoded_buf = Vec::new();

//     // let mut hx: Header = Header {
//     //     parent_hash: (),
//     //     timestamp: (),
//     //     number: (),
//     //     author: (),
//     //     transactions_root: (),
//     //     ommers_hash: (),
//     //     state_root: (),
//     //     receipts_root: (),
//     //     gas_used: (),
//     //     gas_limit: (),
//     //     difficulty: (),
//     //     base_fee: (),
//     // };

//     Header::decode(&hx);

//     //  encoded_buf = Header::decode(&self);
//     let block = Block::decode(bytes_buf).unwrap();
//     let mut encoded_buf = Vec::new();
//     block.encode(&mut encoded_buf);
//     assert_eq!(bytes[..], encoded_buf);
// }
