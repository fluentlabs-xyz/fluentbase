#![allow(dead_code)]
extern crate alloc;
extern crate fluentbase_sdk;
use alloc::vec::Vec;
use fluentbase_sdk::{
    basic_entrypoint,
    derive::{router, Contract},
    Address,
    Bytes,
    SharedAPI,
    I256,
    U256,
};

#[derive(Contract)]
struct App<SDK> {
    sdk: SDK,
}

pub trait SolidityTypesAPI {
    // Test various Solidity types mapping to Rust types
    fn address_test(&self, addr: Address) -> Address;
    fn bytes_test(&self, data: Bytes) -> Bytes;
    fn fixed_bytes_test(&self, data: [u8; 32]) -> [u8; 32];
    fn uint256_test(&self, value: U256) -> U256;
    fn int256_test(&self, value: I256) -> I256;
    fn bool_test(&self, value: bool) -> bool;
    fn array_test(&self, values: Vec<U256>) -> Vec<U256>;
    fn multiple_params(&self, addr: Address, amount: U256, data: Bytes) -> bool;
    fn complex_return(&self, value: u64) -> (Address, U256, bool);
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> SolidityTypesAPI for App<SDK> {
    #[function_id("addressTest(address)", validate(true))]
    fn address_test(&self, addr: Address) -> Address {
        addr
    }

    #[function_id("bytesTest(bytes)", validate(true))]
    fn bytes_test(&self, data: Bytes) -> Bytes {
        data
    }

    #[function_id("fixedBytesTest(bytes32)", validate(true))]
    fn fixed_bytes_test(&self, data: [u8; 32]) -> [u8; 32] {
        data
    }

    #[function_id("uint256Test(uint256)", validate(true))]
    fn uint256_test(&self, value: U256) -> U256 {
        value
    }

    #[function_id("int256Test(int256)", validate(true))]
    fn int256_test(&self, value: I256) -> I256 {
        value
    }

    #[function_id("boolTest(bool)", validate(true))]
    fn bool_test(&self, value: bool) -> bool {
        value
    }

    #[function_id("arrayTest(uint256[])", validate(true))]
    fn array_test(&self, values: Vec<U256>) -> Vec<U256> {
        values
    }

    #[function_id("multipleParams(address,uint256,bytes)", validate(true))]
    fn multiple_params(&self, addr: Address, amount: U256, data: Bytes) -> bool {
        !addr.is_zero() && !amount.is_zero() && !data.is_empty()
    }

    #[function_id("complexReturn(uint64)", validate(true))]
    fn complex_return(&self, value: u64) -> (Address, U256, bool) {
        (Address::default(), U256::from(value), true)
    }
}

impl<SDK: SharedAPI> App<SDK> {
    pub fn deploy(&self) {}
}

basic_entrypoint!(App);

fn main() {}
