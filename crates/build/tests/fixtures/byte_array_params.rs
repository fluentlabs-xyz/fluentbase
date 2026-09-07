// tests/fixtures/byte_array_params.rs
// Test case: byte arrays publish `uint8[N]`, fixed bytes publish `bytesN`

#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{basic_entrypoint, derive::router, FixedBytes, SharedAPI, B256};

#[derive(Default)]
pub struct ByteArrayParams<SDK> {
    sdk: SDK,
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> ByteArrayParams<SDK> {
    /// `[u8; N]` is encoded one word per element, so it is published as `uint8[N]`
    pub fn set_tag(&mut self, tag: [u8; 4], payload: [u8; 32]) -> [u8; 32] {
        let _ = tag;
        payload
    }

    /// `FixedBytes<N>` and the `B*` aliases are one right-padded word: `bytesN`
    pub fn set_hash(&mut self, hash: B256, short: FixedBytes<4>) -> B256 {
        let _ = short;
        hash
    }
}

basic_entrypoint!(ByteArrayParams);
