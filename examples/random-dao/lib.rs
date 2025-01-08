#![cfg_attr(target_arch = "wasm32", no_std)]
#![allow(dead_code)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    basic_entrypoint,
    derive::{router, solidity_storage, Contract},
    Address,
    ContractContextReader,
    SharedAPI,
    B256,
    U256,
};

#[derive(Contract)]
struct RANDOMDAO<SDK> {
    sdk: SDK,
}

pub trait RANDOMDAOAPI {
    fn commit(&mut self, hash: B256);
    fn reveal(&mut self, value: U256);
    fn random_with_peer(&self, peer: Address) -> U256;
}

solidity_storage! {
    mapping(Address => B256) Hashes;
    mapping(Address => U256) Values;
    mapping(Address => U256) Revealed;
}

impl<SDK: SharedAPI> RANDOMDAO<SDK> {
    fn deploy(&mut self) {}
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> RANDOMDAOAPI for RANDOMDAO<SDK> {
    fn commit(&mut self, hash: B256) {
        let caller = self.sdk.context().contract_caller();
        let revealed = Revealed::get(&mut self.sdk, caller);
        if revealed != U256::from(0) {
            panic!("committing a new value after revealing the previous one is not allowed");
        }
        Hashes::set(&mut self.sdk, caller, hash);
    }
    fn reveal(&mut self, value: U256) {
        let caller = self.sdk.context().contract_caller();
        let hash = SDK::keccak256(&value.to_be_bytes::<32>());
        let committed = Hashes::get(&mut self.sdk, caller);
        if hash != committed {
            panic!("passed value is not corresponding to committed hash");
        }
        Revealed::set(&mut self.sdk, caller, U256::from(1));
        Values::set(&mut self.sdk, caller, value);
    }
    fn random_with_peer(&self, peer: Address) -> U256 {
        let caller = self.sdk.context().contract_caller();
        let caller_revealed = Revealed::get(&self.sdk, caller) == U256::from(1);
        let peer_revealed = Revealed::get(&self.sdk, peer) == U256::from(1);
        if !(caller_revealed && peer_revealed) {
            panic!("both caller and peer must reveal their values");
        }
        let caller_value = Values::get(&self.sdk, caller);
        let peer_value = Values::get(&self.sdk, peer);
        return caller_value ^ peer_value;
    }
}

basic_entrypoint!(RANDOMDAO);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{
        journal::{JournalState, JournalStateBuilder},
        runtime::TestingContext,
        ContextFreeNativeAPI,
        ContractContext,
    };
    use hex_literal::hex;

    fn with_caller(caller: Address) -> JournalState<TestingContext> {
        JournalStateBuilder::default()
            .with_contract_context(ContractContext {
                caller: caller,
                ..Default::default()
            })
            .with_devnet_genesis()
            .build(TestingContext::empty())
    }

    #[test]
    fn test_zero_hash() {
        // let native_sdk = TestingContext::empty().with_input("Hello, World");
        // let sdk = JournalState::empty(native_sdk.clone());
        let data = U256::from(0).to_be_bytes::<32>();
        let hash = JournalState::<TestingContext>::keccak256(&data);
        assert_eq!(
            hash,
            B256::new(hex!(
                "290decd9548b62a8d60345a988386fc84ba6bc95484008f6362f93160ef3e563"
            ))
        );
    }

    #[test]
    #[should_panic]
    fn test_zero_hash_commit() {
        let mut caller = RANDOMDAO::new(with_caller(Address::from(hex!(
            "f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
        ))));
        let hash = B256::new(hex!(
            "290decd9548b62a8d60345a988386fc84ba6bc95484008f6362f93160ef3e563"
        ));
        caller.commit(hash);
        caller.reveal(U256::from(0));
        let another_hash = B256::new(hex!(
            "380decd9548b62a8d60345a988386fc84ba6bc95484008f6362f93160ef3e5aa"
        ));
        caller.commit(another_hash);
    }
}
