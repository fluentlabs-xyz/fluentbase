#![cfg_attr(target_arch = "wasm32", no_std)]

use fluentbase_sdk::{
    basic_entrypoint,
    derive::{router, signature, Contract},
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
    fn fvm_withdraw(&mut self, msg: &[u8]);
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> RouterAPI for FvmLoaderEntrypoint<SDK> {
    #[signature("function fvm_deposit(bytes msg, address caller);")]
    fn fvm_deposit(&mut self, msg: &[u8], caller: Address) {
        let msg = "fvm_deposit";
        self.sdk.write(&msg.as_bytes());
    }

    fn fvm_withdraw(&mut self, msg: &[u8]) {
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

        let native_sdk = TestingContext::empty().with_input(call_fvm_deposit.abi_encode());
        let sdk = JournalState::empty(native_sdk.clone());

        let mut contract = FvmLoaderEntrypoint::new(sdk);

        contract.deploy();

        contract.main();

        let output = native_sdk.take_output();
        assert_eq!(&output, &msg);
    }

    #[test]
    fn test_encode_input() {
        let caller = Address::default();
        let msg = "Hello World!".as_bytes();

        let call_fvm_deposit = fvmDepositCall {
            msg: msg.to_vec(),
            caller,
        };

        let input = call_fvm_deposit.abi_encode();

        println!("input: {:?}", input);

        let decoded = fvmDepositCall::abi_decode(&input, false);

        assert_eq!(decoded.unwrap().msg, msg);
    }

    #[test]
    fn test_decode_input() {
        let input = [
            208, 55, 105, 34, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 64, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 12, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 72, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 101, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 108, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            108, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 111, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 32, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 87, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 111, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 114, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 108, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 100, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 33,
        ];

        let decoded = fvmDepositCall::abi_decode(&input[4..], false);
        println!("Decoded: {:?}", decoded.err());
    }

    #[test]
    fn test_signature_works() {
        let caller = Address::default();
        let msg = "Hello World!".as_bytes();
        // FIXME: we can't use fvmDepositCall structure here, because it was created for method,
        // // signature. So probably we would like to create

        // the problem is here. We need to build correct input for the contract.
        // probably we need to provide some tooling for this part
        //
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
