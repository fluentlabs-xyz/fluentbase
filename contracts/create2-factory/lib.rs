#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

extern crate alloc;

use fluentbase_sdk::{
    basic_entrypoint,
    calc_create2_address,
    derive::{router, Contract},
    Address, B256, Bytes, ContextReader, SharedAPI, U256,
};

/// Native CREATE2 factory system contract.
///
/// Exposes deterministic CREATE2 deployment helpers over the shared Fluentbase
/// runtime create syscall surface.
#[derive(Contract)]
struct App<SDK> {
    sdk: SDK,
}

/// Public router surface for the Create2Factory contract.
pub trait Create2FactoryTr {
    /// Deploy a child contract using CREATE2.
    ///
    /// - `salt` participates in deterministic address derivation
    /// - `init_code` is passed to CREATE2 syscall
    /// - returns deployed address on success
    fn deploy_create2(&mut self, salt: U256, init_code: Bytes) -> Address;

    /// Compute the deterministic CREATE2 address using factory address,
    /// salt and init-code hash.
    fn compute_create2_address(&self, salt: U256, init_code_hash: B256) -> Address;
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> Create2FactoryTr for App<SDK> {
    #[fluentbase_sdk::derive::function_id("deployCreate2(uint256,bytes)")]
    fn deploy_create2(&mut self, salt: U256, init_code: Bytes) -> Address {
        let result = self.sdk.create(Some(salt), &U256::ZERO, &init_code).unwrap();
        parse_address_from_create_output(&result)
            .unwrap_or_else(|| panic!("create2-factory: invalid create2 output"))
    }

    #[fluentbase_sdk::derive::function_id("computeCreate2Address(uint256,bytes32)")]
    fn compute_create2_address(&self, salt: U256, init_code_hash: B256) -> Address {
        calc_create2_address(&self.sdk.context().contract_address(), &salt, &init_code_hash)
    }
}

impl<SDK: SharedAPI> App<SDK> {
    /// Constructor entrypoint for system contract deployment.
    pub fn deploy(&self) {
        // System contract, constructor is not expected to be called.
    }
}

/// Parses CREATE/CREATE2 syscall return bytes into an EVM address.
///
/// Runtime may return either:
/// - 20-byte raw address, or
/// - 32-byte ABI word with address in the low 20 bytes.
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
