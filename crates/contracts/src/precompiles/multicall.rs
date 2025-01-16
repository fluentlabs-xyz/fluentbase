#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;
extern crate fluentbase_sdk;

use alloc::vec::Vec;
use fluentbase_sdk::{
    alloc_slice,
    basic_entrypoint,
    bytes::Buf,
    codec::{
        bytes::{BufMut, BytesMut},
        encoder::SolidityABI,
    },
    derive::Contract,
    Bytes,
    ContractContextReader,
    SharedAPI,
};

// Selector for "multicall(bytes[])" - 0xac9650d8
const MULTICALL_SELECTOR: [u8; 4] = [0xac, 0x96, 0x50, 0xd8];

#[derive(Contract)]
struct Multicall<SDK> {
    sdk: SDK,
}

impl<SDK: SharedAPI> Multicall<SDK> {
    fn deploy(&self) {
        // any custom deployment logic here
    }

    fn main(&mut self) {
        let input_length = self.sdk.input_size();
        if input_length < 4 {
            panic!("insufficient input length for method selector");
        }

        // Read full input data
        let mut call_data = alloc_slice(input_length as usize);
        self.sdk.read(&mut call_data, 0);

        // Split into selector and parameters
        let (selector, params) = call_data.split_at(4);

        // Early return on invalid selector
        if selector != MULTICALL_SELECTOR {
            panic!("unsupported method selector");
        }

        // Prepare buffer for decoding - adding offset since Vec<Bytes> is dynamic
        let mut combined_buf = BytesMut::new();
        combined_buf.put_slice(&fluentbase_sdk::U256::from(32).to_be_bytes::<32>());
        combined_buf.put_slice(params);

        // Decode parameters into Vec<Bytes>
        let (data,) = match SolidityABI::<(Vec<Bytes>,)>::decode(&combined_buf.freeze(), 0) {
            Ok(decoded) => decoded,
            Err(err) => panic!("Failed to decode input parameters: {:?}", err),
        };

        // Get contract address for delegate calls
        let target_addr = self.sdk.context().contract_address();
        let mut results = Vec::with_capacity(data.len());

        // Execute each call
        for call_data in data {
            let chunk = call_data.chunk();
            let (output, exit_code) = self.sdk.delegate_call(target_addr, chunk, 0);

            if exit_code != 0 {
                panic!("Multicall: delegate call failed");
            }

            results.push(output);
        }

        // Encode results for return
        let mut buf = BytesMut::new();
        SolidityABI::encode(&(results,), &mut buf, 0).expect("Failed to encode output");
        let encoded_output = buf.freeze();

        // Remove offset from encoded output since caller expects only data
        let clean_output = encoded_output[32..].to_vec();
        self.sdk.write(&clean_output);
    }
}

basic_entrypoint!(Multicall);
