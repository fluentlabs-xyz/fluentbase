#![cfg_attr(target_arch = "wasm32", no_std)]

use fluentbase_sdk::{
    basic_entrypoint,
    derive::{function_id, router, signature, Contract},
    Address,
    ExitCode,
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
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> RouterAPI for FvmLoaderEntrypoint<SDK> {
    #[signature("function fvm_deposit(bytes msg, address caller);")]
    fn fvm_deposit(&mut self, msg: &[u8], caller: Address) {
        let msg = "fvm_deposit";
        self.sdk.write(&msg.as_bytes());
    }

    // fn_id = 212u8,173u8,13u8,159u8
    // NOTE: signature invalid - should be "fvmWithdraw(uint8[])" (without semicolon)
    #[signature("fvmWithdraw(uint8[]);", false)]
    fn fvm_withdraw(&mut self, msg: &mut [u8]) {
        self.sdk.write(msg);
    }

    #[function_id("0x12345678")]
    fn fvm_example(&mut self, msg: &[u8]) {
        self.sdk.write(msg);
    }
}

impl<SDK: SharedAPI> FvmLoaderEntrypoint<SDK> {
    pub fn deploy(&mut self) {}
}

basic_entrypoint!(FvmLoaderEntrypoint);

#[cfg(test)]
mod tests {
    use super::*;
    use core::ops::Add;
    use fluentbase_sdk::{
        address,
        journal::{JournalState, JournalStateBuilder},
        runtime::TestingContext,
        Address,
        ContractContext,
    };

    fn with_test_input<T: Into<Vec<u8>>>(
        input: T,
        caller: Option<Address>,
    ) -> JournalState<TestingContext> {
        JournalStateBuilder::default()
            .with_contract_context(ContractContext {
                caller: caller.unwrap_or_default(),
                ..Default::default()
            })
            .with_devnet_genesis()
            .build(TestingContext::empty().with_input(input))
    }

    #[test]
    fn test_contract_works() {
        let msg = vec![1, 2, 3, 4, 5];
        let call_fvm_deposit = fvmWithdrawCall { msg: msg.clone() };

        let mut input = call_fvm_deposit.abi_encode();
        let real_func_id = [212u8, 173u8, 13u8, 159u8];

        input[0..4].copy_from_slice(&real_func_id);

        let native_sdk = TestingContext::empty().with_input(input);
        let sdk = JournalState::empty(native_sdk.clone());

        let mut contract = FvmLoaderEntrypoint::new(sdk);

        contract.deploy();

        contract.main();

        let output = native_sdk.take_output();
        assert_eq!(&output, &msg);
    }

    #[test]
    fn test_signature_works() {
        let caller = Address::default();
        let msg = "Hello World!".as_bytes();

        let call_fvm_deposit = fvmDepositCall {
            msg: msg.to_vec(),
            caller,
        };

        let mut input = call_fvm_deposit.abi_encode();

        let fn_id_from_signature_attr = [153u8, 65u8, 183u8, 19u8];
        println!("before: {:?}", input);

        input[0..4].copy_from_slice(&fn_id_from_signature_attr);

        println!("after: {:?}", input);

        let native_sdk = TestingContext::empty().with_input(input);
        let sdk = JournalState::empty(native_sdk.clone());

        let mut contract = FvmLoaderEntrypoint::new(sdk);

        contract.deploy();

        contract.main();

        let output = native_sdk.take_output();
        let expected_output = "fvm_deposit".as_bytes();
        assert_eq!(&output, &expected_output);
    }
}
