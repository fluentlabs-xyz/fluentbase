#![cfg_attr(target_arch = "wasm32", no_std, no_main)]

mod storage;

use crate::storage::Balance;
use fluentbase_sdk::{
    byteorder::{ByteOrder, LittleEndian},
    derive::{derive_evm_error, derive_keccak256, derive_keccak256_id},
    entrypoint,
    Address,
    SharedAPI,
    B256,
    U256,
};

pub fn deploy_entry(sdk: impl SharedAPI) {
    sdk.input_size();
}

#[allow(unused)]
const ERR_MALFORMED_INPUT: u32 = derive_evm_error!("MalformedInput()");
#[allow(unused)]
const ERR_INSUFFICIENT_BALANCE: u32 = derive_evm_error!("InsufficientBalance()");
#[allow(unused)]

const SIG_SYMBOL: u32 = derive_keccak256_id!("symbol()");
#[allow(unused)]
const SIG_NAME: u32 = derive_keccak256_id!("name()");
#[allow(unused)]
const SIG_DECIMALS: u32 = derive_keccak256_id!("decimals()");
#[allow(unused)]
const SIG_TOTAL_SUPPLY: u32 = derive_keccak256_id!("totalSupply()");
#[allow(unused)]
const SIG_BALANCE_OF: u32 = derive_keccak256_id!("balanceOf(address)");
#[allow(unused)]
const SIG_TRANSFER: u32 = derive_keccak256_id!("transfer(address,uint256)");
#[allow(unused)]
const SIG_ALLOWANCE: u32 = derive_keccak256_id!("allowance(address)");
#[allow(unused)]
const SIG_APPROVE: u32 = derive_keccak256_id!("approve(address,uint256)");
#[allow(unused)]
const SIG_TRANSFER_FROM: u32 = derive_keccak256_id!("transferFrom(address,address,uint256)");
#[allow(unused)]

const EVENT_TRANSFER: B256 = B256::new(derive_keccak256!("Transfer(address,address,uint256)"));
#[allow(unused)]
const EVENT_APPROVAL: B256 = B256::new(derive_keccak256!("Approval(address,address,uint256)"));

fn symbol(mut sdk: impl SharedAPI, _input: &[u8]) {
    sdk.write("Universal Token".as_bytes());
}
fn name(mut sdk: impl SharedAPI, _input: &[u8]) {
    sdk.write("UT".as_bytes());
}
fn decimals(mut sdk: impl SharedAPI, _input: &[u8]) {
    let output = U256::from(18).to_be_bytes::<32>();
    sdk.write(&output);
}

fn transfer(mut sdk: impl SharedAPI, input: &[u8]) {
    let Ok(recipient) = Address::try_from(&input[..20]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Some(amount) = U256::try_from_be_slice(&input[20..20 + 32]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let mut from = Address::ZERO;
    // TODO(dmitry123): "replace 321 constant"
    sdk.read_context(from.as_mut_slice(), 321);
    if !Balance::subtract(&mut sdk, from, amount) {
        sdk.evm_exit(ERR_INSUFFICIENT_BALANCE);
    }
    Balance::add(&mut sdk, recipient, amount);
    // sdk.emit_log(&[], &[]);
    let result = U256::from(1).to_be_bytes::<32>();
    sdk.write(&result);
}

pub fn main_entry(mut sdk: impl SharedAPI) {
    let input_size = sdk.input_size();
    if input_size < 4 {
        sdk.evm_exit(ERR_INSUFFICIENT_BALANCE);
    }
    let (sig, input) = sdk.input().split_at(4);
    let signature = LittleEndian::read_u32(sig);
    match signature {
        SIG_SYMBOL => symbol(sdk, input),
        SIG_NAME => name(sdk, input),
        SIG_TRANSFER => transfer(sdk, input),
        SIG_DECIMALS => decimals(sdk, input),
        _ => {
            sdk.evm_exit(ERR_INSUFFICIENT_BALANCE);
        }
    }
}

entrypoint!(main_entry, deploy_entry);

#[cfg(test)]
mod tests {

    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
