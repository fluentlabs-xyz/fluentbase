#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate fluentbase_sdk;

use alloc::vec::Vec;
use fluentbase_sdk::{
    bytes::Buf,
    codec::{bytes::BytesMut, encoder::SolidityABI},
    entrypoint, Bytes, ContextReader, SharedAPI, SyscallResult,
};

/// A selector for "multicall(bytes[])" - 0xac9650d8
const MULTICALL_SELECTOR: [u8; 4] = [0xac, 0x96, 0x50, 0xd8];

pub fn main_entry(mut sdk: impl SharedAPI) {
    // Read full input data
    let input_length = sdk.input_size();
    assert!(input_length >= 4, "multicall: insufficient input length");
    let call_data = sdk.bytes_input();

    // Split into selector and parameters
    let (selector, params) = call_data.split_at(4);
    assert_eq!(
        selector, MULTICALL_SELECTOR,
        "multicall: invalid method selector"
    );

    // Decode parameters into Vec<Bytes>
    let data = SolidityABI::<Vec<Bytes>>::decode(&params, 0)
        .unwrap_or_else(|_| panic!("multicall: can't decode input parameters"));

    // Get contract address for delegate calls
    let target_addr = sdk.context().contract_address();
    let mut results = Vec::with_capacity(data.len());

    // Execute each call
    for call_data in data {
        let chunk = call_data.chunk();
        let result = sdk.delegate_call(target_addr, chunk, None);
        if !SyscallResult::is_ok(result.status) {
            panic!("multicall: delegate call failed");
        }
        results.push(result.data);
    }

    // Encode results for return
    let mut buf = BytesMut::new();
    SolidityABI::encode(&(results,), &mut buf, 0)
        .unwrap_or_else(|_| panic!("multicall: can't decode input parameters"));
    let encoded_output = buf.freeze();

    // Remove offset from encoded output since caller expects only data
    let clean_output = encoded_output[32..].to_vec();
    sdk.write(&clean_output);
}

entrypoint!(main_entry);
