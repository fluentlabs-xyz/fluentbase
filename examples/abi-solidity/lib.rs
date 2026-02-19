#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{bytes::BytesMut, codec::SolidityABI, entrypoint, SharedAPI, U256};

pub fn main_entry(mut sdk: impl SharedAPI) {
    let input = sdk.bytes_input();
    let value: U256 =
        SolidityABI::decode(&input, 0).unwrap_or_else(|_| panic!("malformed ABI input"));
    let value = value * U256::from(2);
    let mut output = BytesMut::new();
    SolidityABI::encode(&value, &mut output, 0).unwrap_or_else(|_| panic!("decode ABI failed"));
    sdk.write(&output);
}

entrypoint!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_testing::TestingContextImpl;
    use hex_literal::hex;

    #[test]
    fn test_contract_works() {
        let sdk = TestingContextImpl::default().with_input(hex!(
            "000000000000000000000000000000000000000000000000000000000000007b"
        ));
        main_entry(sdk.clone());
        let output = sdk.take_output();
        let value = U256::from_be_slice(&output);
        assert_eq!(value, U256::from(246));
    }
}
