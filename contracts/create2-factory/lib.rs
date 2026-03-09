#![cfg_attr(target_arch = "wasm32", no_std, no_main)]

extern crate alloc;

use fluentbase_sdk::{
    basic_entrypoint,
    derive::{function_id, router, Contract, Event},
    calc_create2_address, Address, Bytes, ContextReader, SharedAPI, U256, B256,
};

#[derive(Contract)]
struct App<SDK> {
    sdk: SDK,
}

#[derive(Event, Debug)]
struct Deployed {
    #[indexed]
    deployer: Address,
    #[indexed]
    deployed: Address,
    salt: U256,
    is_create2: bool,
}

pub trait Create2FactoryTr {
    fn deploy_create(&mut self, init_code: Bytes) -> Address;

    fn deploy_create2(&mut self, salt: U256, init_code: Bytes) -> Address;

    fn compute_create2_address(&self, salt: U256, init_code_hash: B256) -> Address;
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> Create2FactoryTr for App<SDK> {
    #[function_id("deployContract(bytes)")]
    fn deploy_create(&mut self, init_code: Bytes) -> Address {
        let result = self.sdk.create(None, &U256::ZERO, &init_code).unwrap();
        let deployed = parse_address_from_create_output(&result)
            .unwrap_or_else(|| panic!("create2-factory: invalid create output"));
        let deployer = self.sdk.context().contract_caller();
        Deployed {
            deployer,
            deployed,
            salt: U256::ZERO,
            is_create2: false,
        }
        .emit(&mut self.sdk);
        deployed
    }

    #[function_id("deployContract2(uint256,bytes)")]
    fn deploy_create2(&mut self, salt: U256, init_code: Bytes) -> Address {
        let result = self.sdk.create(Some(salt), &U256::ZERO, &init_code).unwrap();
        let deployed = parse_address_from_create_output(&result)
            .unwrap_or_else(|| panic!("create2-factory: invalid create2 output"));
        let deployer = self.sdk.context().contract_caller();
        Deployed {
            deployer,
            deployed,
            salt,
            is_create2: true,
        }
        .emit(&mut self.sdk);
        deployed
    }

    #[function_id("computeCreate2Address(uint256,bytes32)")]
    fn compute_create2_address(&self, salt: U256, init_code_hash: B256) -> Address {
        calc_create2_address(
            &self.sdk.context().contract_address(),
            &salt,
            &init_code_hash,
        )
    }
}

impl<SDK: SharedAPI> App<SDK> {
    pub fn deploy(&self) {
        // System contract, constructor is not expected to be called.
    }
}

fn parse_address_from_create_output(output: &[u8]) -> Option<Address> {
    match output.len() {
        20 => Some(Address::from_slice(output)),
        32 => Some(Address::from_slice(&output[12..32])),
        _ => None,
    }
}

basic_entrypoint!(App);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{address, B256};

    #[test]
    fn parse_address_20_bytes() {
        let addr = address!("0x1111111111111111111111111111111111111111");
        let parsed = parse_address_from_create_output(addr.as_slice()).unwrap();
        assert_eq!(parsed, addr);
    }

    #[test]
    fn parse_address_32_bytes() {
        let addr = address!("0x2222222222222222222222222222222222222222");
        let mut data = [0u8; 32];
        data[12..32].copy_from_slice(addr.as_slice());
        let parsed = parse_address_from_create_output(&data).unwrap();
        assert_eq!(parsed, addr);
    }

    #[test]
    fn parse_address_invalid_size() {
        assert!(parse_address_from_create_output(&[1u8; 19]).is_none());
        assert!(parse_address_from_create_output(&[1u8; 21]).is_none());
    }

    #[test]
    fn compute_create2_matches_address_helper() {
        let deployer = address!("0x3333333333333333333333333333333333333333");
        let salt = U256::from(123456u64);
        let init_code_hash = B256::repeat_byte(0x44);
        let expected = deployer.create2(salt.to_be_bytes::<32>(), init_code_hash);
        let actual = calc_create2_address(&deployer, &salt, &init_code_hash);
        assert_eq!(actual, expected);
    }
}
