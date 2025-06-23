#![cfg_attr(target_arch = "wasm32", no_std, no_main)]

mod storage;
use crate::storage::{Allowance, Balance, Settings, SIG_SIZE_BYTES, U256_SIZE_BYTES};
use fluentbase_sdk::{
    byteorder::{ByteOrder, LittleEndian},
    derive::{derive_evm_error, derive_keccak256, derive_keccak256_id},
    entrypoint,
    Address,
    ContextReader,
    SharedAPI,
    B256,
    U256,
};

pub fn deploy_entry(mut sdk: impl SharedAPI) {
    let input = sdk.input();
    let minter = sdk.context().contract_caller();
    // need params: owner, owner_balance (total_supply), decimals, name, symbol
    const TOTAL_SUPPLY_OFFSET: usize = 0;
    const DECIMALS_OFFSET: usize = TOTAL_SUPPLY_OFFSET + U256_SIZE_BYTES;
    const NAME_OFFSET: usize = DECIMALS_OFFSET + 1; // 1 for decimals as u8 repr
    const INPUT_LEN_MIN: usize =
        NAME_OFFSET + Settings::SHORT_STR_LEN_MIN + Settings::SHORT_STR_BYTE_REPR_LEN_MIN + 1;

    if input.len() < INPUT_LEN_MIN {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    }
    let Some(total_supply) =
        U256::try_from_be_slice(&input[TOTAL_SUPPLY_OFFSET..TOTAL_SUPPLY_OFFSET + U256_SIZE_BYTES])
    else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let decimals = U256::from(input[DECIMALS_OFFSET]);
    let name = if let Ok(name) = Settings::short_str_try_from_slice_repr(&input[NAME_OFFSET..]) {
        name
    } else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let symbol_offset = NAME_OFFSET + 1 + name.len();
    if input.len() <= symbol_offset {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    }
    let symbol =
        if let Ok(symbol) = Settings::short_str_try_from_slice_repr(&input[symbol_offset..]) {
            symbol
        } else {
            sdk.evm_exit(ERR_MALFORMED_INPUT);
        };

    // setup settings
    Settings::decimals_set(&mut sdk, decimals);
    Settings::symbol_set(&mut sdk, symbol);
    Settings::name_set(&mut sdk, name);
    // setup initial user
    Balance::add(&mut sdk, minter, total_supply);
}

const ERR_MALFORMED_INPUT: u32 = derive_evm_error!("MalformedInput()");
const ERR_INSUFFICIENT_BALANCE: u32 = derive_evm_error!("InsufficientBalance()");

const SIG_SYMBOL: u32 = derive_keccak256_id!("symbol()");
const SIG_NAME: u32 = derive_keccak256_id!("name()");
const SIG_DECIMALS: u32 = derive_keccak256_id!("decimals()");
const SIG_TOTAL_SUPPLY: u32 = derive_keccak256_id!("totalSupply()");
const SIG_BALANCE_OF: u32 = derive_keccak256_id!("balanceOf(address)");
const SIG_TRANSFER: u32 = derive_keccak256_id!("transfer(address,uint256)");
const SIG_TRANSFER_FROM: u32 = derive_keccak256_id!("transferFrom(address,address,uint256)");
const SIG_ALLOWANCE: u32 = derive_keccak256_id!("allowance(address)");
const SIG_APPROVE: u32 = derive_keccak256_id!("approve(address,uint256)");

const EVENT_TRANSFER: B256 = B256::new(derive_keccak256!("Transfer(address,address,uint256)"));
const EVENT_APPROVAL: B256 = B256::new(derive_keccak256!("Approval(address,address,uint256)"));

fn symbol(mut sdk: impl SharedAPI, _input: &[u8]) {
    sdk.write(Settings::symbol(&sdk));
}
fn name(mut sdk: impl SharedAPI, _input: &[u8]) {
    sdk.write(Settings::name(&sdk));
}
fn decimals(mut sdk: impl SharedAPI, _input: &[u8]) {
    let output = Settings::decimals_get(&sdk).to_be_bytes::<U256_SIZE_BYTES>();
    sdk.write(&output);
}

fn transfer(mut sdk: impl SharedAPI, input: &[u8]) {
    let from = sdk.context().contract_caller();
    const TO_OFFSET: usize = 0;
    const AMOUNT_OFFSET: usize = TO_OFFSET + Address::len_bytes();
    let Ok(to) = Address::try_from(&input[TO_OFFSET..TO_OFFSET + Address::len_bytes()]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Some(amount) =
        U256::try_from_be_slice(&input[AMOUNT_OFFSET..AMOUNT_OFFSET + U256_SIZE_BYTES])
    else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    if !Balance::subtract(&mut sdk, from, amount) {
        sdk.evm_exit(ERR_INSUFFICIENT_BALANCE);
    }
    Balance::add(&mut sdk, to, amount);
    sdk.emit_log(
        &[
            EVENT_TRANSFER,
            // TODO is it optimal conversion?
            B256::left_padding_from(from.as_slice()),
            // TODO is it optimal conversion?
            B256::left_padding_from(to.as_slice()),
        ],
        &amount.to_be_bytes::<U256_SIZE_BYTES>(),
    );
    let result = U256::from(1).to_be_bytes::<U256_SIZE_BYTES>();
    sdk.write(&result);
}

fn transfer_from(mut sdk: impl SharedAPI, input: &[u8]) {
    let spender = sdk.context().contract_caller();
    const FROM_OFFSET: usize = 0;
    const TO_OFFSET: usize = FROM_OFFSET + Address::len_bytes();
    const AMOUNT_OFFSET: usize = TO_OFFSET + Address::len_bytes();
    let Ok(to) = Address::try_from(&input[TO_OFFSET..TO_OFFSET + Address::len_bytes()]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Some(amount) =
        U256::try_from_be_slice(&input[AMOUNT_OFFSET..AMOUNT_OFFSET + U256_SIZE_BYTES])
    else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let from = {
        let Ok(from) = Address::try_from(&input[FROM_OFFSET..FROM_OFFSET + Address::len_bytes()])
        else {
            sdk.evm_exit(ERR_MALFORMED_INPUT);
        };
        if Allowance::subtract(&mut sdk, from, spender, amount) {
            sdk.evm_exit(ERR_INSUFFICIENT_BALANCE);
        }
        from
    };
    if !Balance::subtract(&mut sdk, from, amount) {
        sdk.evm_exit(ERR_INSUFFICIENT_BALANCE);
    }
    Balance::add(&mut sdk, to, amount);
    sdk.emit_log(
        &[
            EVENT_TRANSFER,
            // TODO is it optimal conversion?
            B256::left_padding_from(from.as_slice()),
            // TODO is it optimal conversion?
            B256::left_padding_from(to.as_slice()),
        ],
        &amount.to_be_bytes::<U256_SIZE_BYTES>(),
    );
    let result = U256::from(1).to_be_bytes::<U256_SIZE_BYTES>();
    sdk.write(&result);
}

fn approve(mut sdk: impl SharedAPI, input: &[u8]) {
    const OWNER_OFFSET: usize = 0;
    const SPENDER_OFFSET: usize = OWNER_OFFSET + Address::len_bytes();
    const AMOUNT_OFFSET: usize = SPENDER_OFFSET + Address::len_bytes();
    let Ok(owner) = Address::try_from(&input[OWNER_OFFSET..OWNER_OFFSET + Address::len_bytes()])
    else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(spender) =
        Address::try_from(&input[SPENDER_OFFSET..SPENDER_OFFSET + Address::len_bytes()])
    else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Some(amount) =
        U256::try_from_be_slice(&input[AMOUNT_OFFSET..AMOUNT_OFFSET + size_of::<U256>()])
    else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    Allowance::update(&mut sdk, owner, spender, amount);
    sdk.emit_log(&[EVENT_APPROVAL], &[]);
    let result = U256::from(1).to_be_bytes::<U256_SIZE_BYTES>();
    sdk.write(&result);
}

fn allow(mut sdk: impl SharedAPI, input: &[u8]) {
    const OWNER_OFFSET: usize = 0;
    const SPENDER_OFFSET: usize = OWNER_OFFSET + Address::len_bytes();
    let Ok(owner) = Address::try_from(&input[OWNER_OFFSET..OWNER_OFFSET + Address::len_bytes()])
    else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(spender) =
        Address::try_from(&input[SPENDER_OFFSET..SPENDER_OFFSET + Address::len_bytes()])
    else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let amount = Allowance::get_current(&mut sdk, owner, spender);
    sdk.write(&amount.to_be_bytes::<U256_SIZE_BYTES>());
}

fn total_supply(mut sdk: impl SharedAPI, _input: &[u8]) {
    let result = Settings::total_supply_get(&sdk);
    sdk.write(&result.to_be_bytes::<U256_SIZE_BYTES>())
}

fn balance_of(mut sdk: impl SharedAPI, input: &[u8]) {
    let Ok(owner) = Address::try_from(&input[..Address::len_bytes()]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let result = Balance::get_for(&sdk, owner);
    sdk.write(&result.to_be_bytes::<U256_SIZE_BYTES>())
}

pub fn main_entry(mut sdk: impl SharedAPI) {
    let input_size = sdk.input_size();
    if input_size < SIG_SIZE_BYTES as u32 {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    }
    let (sig, input) = sdk.input().split_at(SIG_SIZE_BYTES);
    let signature = LittleEndian::read_u32(sig);
    match signature {
        SIG_SYMBOL => symbol(sdk, input),
        SIG_NAME => name(sdk, input),
        SIG_TRANSFER => transfer(sdk, input),
        SIG_TRANSFER_FROM => transfer_from(sdk, input),
        SIG_APPROVE => approve(sdk, input),
        SIG_DECIMALS => decimals(sdk, input),
        SIG_ALLOWANCE => allow(sdk, input),
        SIG_TOTAL_SUPPLY => total_supply(sdk, input),
        SIG_BALANCE_OF => balance_of(sdk, input),
        _ => {
            sdk.evm_exit(ERR_MALFORMED_INPUT);
        }
    }
}

entrypoint!(main_entry, deploy_entry);

#[cfg(test)]
mod tests {
    use crate::{
        storage::U256_SIZE_BYTES,
        ERR_INSUFFICIENT_BALANCE,
        ERR_MALFORMED_INPUT,
        SIG_ALLOWANCE,
        SIG_APPROVE,
        SIG_BALANCE_OF,
        SIG_DECIMALS,
        SIG_NAME,
        SIG_SYMBOL,
        SIG_TOTAL_SUPPLY,
        SIG_TRANSFER,
        SIG_TRANSFER_FROM,
    };
    use fluentbase_sdk::{HashSet, U256};

    #[test]
    fn u256_bytes_size() {
        assert_eq!(size_of::<U256>(), U256_SIZE_BYTES);
    }

    #[test]
    fn check_for_collisions() {
        let values = [
            ERR_MALFORMED_INPUT,
            ERR_INSUFFICIENT_BALANCE,
            SIG_SYMBOL,
            SIG_NAME,
            SIG_DECIMALS,
            SIG_TOTAL_SUPPLY,
            SIG_BALANCE_OF,
            SIG_TRANSFER,
            SIG_TRANSFER_FROM,
            SIG_ALLOWANCE,
            SIG_APPROVE,
        ];
        let values_hashset: HashSet<u32> = values.into();
        assert_eq!(values.len(), values_hashset.len());
    }
}
