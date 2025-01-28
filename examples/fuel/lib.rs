#![cfg_attr(target_arch = "wasm32", no_std)]
#![allow(unused)]

use fluentbase_sdk::{
    basic_entrypoint,
    derive::{function_id, router, Contract},
    Address,
    SharedAPI,
};

#[derive(Contract)]
pub struct FvmLoaderEntrypoint<SDK> {
    sdk: SDK,
}

pub trait RouterAPI {
    fn fvm_deposit(&mut self, msg: &[u8], caller: Address);
    fn fvm_withdraw(&mut self, msg: &mut [u8]);
    fn fvm_example(&mut self, msg: &[u8]);
    fn example(&mut self, msg: fluentbase_sdk::Bytes);
}
extern crate alloc;
use alloc::vec;

#[router(mode = "solidity")]
impl<SDK: SharedAPI> RouterAPI for FvmLoaderEntrypoint<SDK> {
    #[function_id("fvm_deposit(bytes1[],address)", validate(false))]
    fn fvm_deposit(&mut self, msg: &[u8], caller: Address) {
        let msg = "fvm_deposit";
        self.sdk.write(&msg.as_bytes());
    }

    // fn_id = 212u8,173u8,13u8,159u8
    // NOTE: function_id invalid - should be "fvmWithdraw(uint8[])" (without semicolon)
    #[function_id("fvmWithdraw(uint8[])", validate(false))]
    fn fvm_withdraw(&mut self, msg: &mut [u8]) {
        self.sdk.write(msg);
    }

    #[function_id("0x12345678", validate(false))]
    fn fvm_example(&mut self, msg: &[u8]) {
        self.sdk.write(msg);
    }

    fn example(&mut self, msg: fluentbase_sdk::Bytes) {
        self.sdk.write(&msg);
    }
}

impl<SDK: SharedAPI> FvmLoaderEntrypoint<SDK> {
    pub fn deploy(&mut self) {}
}

basic_entrypoint!(FvmLoaderEntrypoint);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{
        journal::{JournalState, JournalStateBuilder},
        runtime::TestingContext,
        Address,
        ContractContextV1,
    };

    fn with_test_input<T: Into<Vec<u8>>>(
        input: T,
        caller: Option<Address>,
    ) -> JournalState<TestingContext> {
        JournalStateBuilder::default()
            .with_contract_context(ContractContextV1 {
                caller: caller.unwrap_or_default(),
                ..Default::default()
            })
            .with_devnet_genesis()
            .build(TestingContext::empty().with_input(input))
    }

    #[test]
    fn test_contract_works() {
        let msg = vec![1, 2, 3, 4, 5];
        let call_fvm_deposit = FvmWithdrawCall::new((msg.clone(),));

        let mut input = call_fvm_deposit.encode();

        let native_sdk = TestingContext::empty().with_input(input);
        let sdk = JournalState::empty(native_sdk.clone());

        let mut contract = FvmLoaderEntrypoint::new(sdk);

        contract.deploy();

        contract.main();

        let output = native_sdk.take_output();
        assert_eq!(&output, &msg);
    }
}
