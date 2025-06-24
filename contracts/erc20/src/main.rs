#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate core;

mod storage;
use crate::storage::{
    fixed_bytes_from_u256,
    u256_from_fixed_bytes,
    Allowance,
    Balance,
    Config,
    Feature,
    InitialSettings,
    Settings,
    ADDRESS_LEN_BYTES,
    SIG_LEN_BYTES,
    U256_LEN_BYTES,
};
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
    let (initial_settings, _) =
        InitialSettings::try_from_slice(&input).expect("failed to decode initial settings");
    assert!(initial_settings.is_valid(), "initial settings invalid");
    let mut config = Config::new();
    for f in initial_settings.features() {
        match f {
            Feature::Meta { name, symbol } => {
                assert!(
                    Settings::symbol_set(&mut sdk, symbol),
                    "meta feature (symbol) invalid"
                );
                assert!(
                    Settings::name_set(&mut sdk, name),
                    "meta feature (name) invalid"
                );
            }
            Feature::InitialTokenSupply {
                amount,
                owner,
                decimals,
            } => {
                let amount = u256_from_fixed_bytes(*amount);
                let owner = owner.into();
                Settings::decimals_set(&mut sdk, U256::from(*decimals));
                Settings::total_supply_set(&mut sdk, amount);
                Balance::add(&mut sdk, owner, amount);
            }
            Feature::MintableFunctionality { minter } => {
                config.enable_mintable_plugin(&sdk);
                Settings::minter_set(&mut sdk, &Address::from(minter));
            }
            Feature::Pausable { pauser } => {
                config.enable_pausable_plugin(&sdk);
                Settings::pauser_set(&mut sdk, &Address::from(pauser));
            }
        }
    }
    config.save_flags(&mut sdk);
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
const SIG_MINT: u32 = derive_keccak256_id!("mint(address,uint256)");
const SIG_PAUSE: u32 = derive_keccak256_id!("pause()");
const SIG_UNPAUSE: u32 = derive_keccak256_id!("unpause()");

const EVENT_TRANSFER: B256 = B256::new(derive_keccak256!("Transfer(address,address,uint256)"));
const EVENT_APPROVAL: B256 = B256::new(derive_keccak256!("Approval(address,address,uint256)"));
const EVENT_PAUSED: B256 = B256::new(derive_keccak256!("Paused(address)"));
const EVENT_UNPAUSED: B256 = B256::new(derive_keccak256!("Unpaused(address)"));

fn emit_transfer_event(sdk: &mut impl SharedAPI, from: &Address, to: &Address, amount: &U256) {
    sdk.emit_log(
        &[
            EVENT_TRANSFER,
            B256::left_padding_from(from.as_slice()),
            B256::left_padding_from(to.as_slice()),
        ],
        &fixed_bytes_from_u256(amount),
    );
}

fn emit_pause_event(sdk: &mut impl SharedAPI, pauser: &Address) {
    sdk.emit_log(&[EVENT_PAUSED], pauser.as_slice());
}

fn emit_unpause_event(sdk: &mut impl SharedAPI, pauser: &Address) {
    sdk.emit_log(&[EVENT_UNPAUSED], pauser.as_slice());
}

fn emit_approval_event(
    sdk: &mut impl SharedAPI,
    owner: &Address,
    spender: &Address,
    amount: &U256,
) {
    sdk.emit_log(
        &[
            EVENT_APPROVAL,
            B256::left_padding_from(owner.as_slice()),
            B256::left_padding_from(spender.as_slice()),
        ],
        &fixed_bytes_from_u256(amount),
    );
}

fn symbol(mut sdk: impl SharedAPI, _input: &[u8]) {
    sdk.write(Settings::symbol(&sdk));
}
fn name(mut sdk: impl SharedAPI, _input: &[u8]) {
    sdk.write(Settings::name(&sdk));
}
fn decimals(mut sdk: impl SharedAPI, _input: &[u8]) {
    let output = Settings::decimals_get(&sdk).to_be_bytes::<U256_LEN_BYTES>();
    sdk.write(&output);
}

fn transfer(mut sdk: impl SharedAPI, input: &[u8]) {
    let from = sdk.context().contract_caller();
    const TO_OFFSET: usize = 0;
    const AMOUNT_OFFSET: usize = TO_OFFSET + ADDRESS_LEN_BYTES;
    let Ok(to) = Address::try_from(&input[TO_OFFSET..TO_OFFSET + ADDRESS_LEN_BYTES]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Some(amount) =
        U256::try_from_be_slice(&input[AMOUNT_OFFSET..AMOUNT_OFFSET + U256_LEN_BYTES])
    else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    Balance::send(&mut sdk, from, to, amount);
    emit_transfer_event(&mut sdk, &from, &to, &amount);
    let result = U256::from(1).to_be_bytes::<U256_LEN_BYTES>();
    sdk.write(&result);
}

fn transfer_from(mut sdk: impl SharedAPI, input: &[u8]) {
    let spender = sdk.context().contract_caller();
    const FROM_OFFSET: usize = 0;
    const TO_OFFSET: usize = FROM_OFFSET + ADDRESS_LEN_BYTES;
    const AMOUNT_OFFSET: usize = TO_OFFSET + ADDRESS_LEN_BYTES;
    let Ok(to) = Address::try_from(&input[TO_OFFSET..TO_OFFSET + ADDRESS_LEN_BYTES]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Some(amount) =
        U256::try_from_be_slice(&input[AMOUNT_OFFSET..AMOUNT_OFFSET + U256_LEN_BYTES])
    else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let from = {
        let Ok(from) = Address::try_from(&input[FROM_OFFSET..FROM_OFFSET + ADDRESS_LEN_BYTES])
        else {
            sdk.evm_exit(ERR_MALFORMED_INPUT);
        };
        if Allowance::subtract(&mut sdk, from, spender, amount) {
            sdk.evm_exit(ERR_INSUFFICIENT_BALANCE);
        }
        from
    };
    Balance::send(&mut sdk, from, to, amount);
    emit_transfer_event(&mut sdk, &from, &to, &amount);
    let result = fixed_bytes_from_u256(&U256::from(1));
    sdk.write(&result);
}

fn approve(mut sdk: impl SharedAPI, input: &[u8]) {
    const OWNER_OFFSET: usize = 0;
    const SPENDER_OFFSET: usize = OWNER_OFFSET + ADDRESS_LEN_BYTES;
    const AMOUNT_OFFSET: usize = SPENDER_OFFSET + ADDRESS_LEN_BYTES;
    let Ok(owner) = Address::try_from(&input[OWNER_OFFSET..OWNER_OFFSET + ADDRESS_LEN_BYTES])
    else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(spender) = Address::try_from(&input[SPENDER_OFFSET..SPENDER_OFFSET + ADDRESS_LEN_BYTES])
    else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Some(amount) =
        U256::try_from_be_slice(&input[AMOUNT_OFFSET..AMOUNT_OFFSET + size_of::<U256>()])
    else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    Allowance::update(&mut sdk, owner, spender, amount);
    emit_approval_event(&mut sdk, &owner, &spender, &amount);
    sdk.write(&fixed_bytes_from_u256(&U256::from(1)));
}

fn allow(mut sdk: impl SharedAPI, input: &[u8]) {
    const OWNER_OFFSET: usize = 0;
    const SPENDER_OFFSET: usize = OWNER_OFFSET + ADDRESS_LEN_BYTES;
    let Ok(owner) = Address::try_from(&input[OWNER_OFFSET..OWNER_OFFSET + ADDRESS_LEN_BYTES])
    else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(spender) = Address::try_from(&input[SPENDER_OFFSET..SPENDER_OFFSET + ADDRESS_LEN_BYTES])
    else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let amount = Allowance::get_current(&mut sdk, owner, spender);
    sdk.write(&fixed_bytes_from_u256(&amount));
}

fn total_supply(mut sdk: impl SharedAPI, _input: &[u8]) {
    let result = Settings::total_supply_get(&sdk);
    sdk.write(&fixed_bytes_from_u256(&result))
}

fn balance_of(mut sdk: impl SharedAPI, input: &[u8]) {
    let Ok(owner) = Address::try_from(&input[..ADDRESS_LEN_BYTES]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let result = Balance::get_for(&sdk, owner);
    sdk.write(&fixed_bytes_from_u256(&result))
}

fn mint(mut sdk: impl SharedAPI, input: &[u8]) {
    let mut config = Config::new();
    assert!(
        config.mintable_plugin_enabled(&sdk),
        "mintability not enabled"
    );
    let minter = sdk.context().contract_caller();
    assert_eq!(
        minter,
        Settings::minter_get(&sdk),
        "permission denied: not enough privilege"
    );
    if config.pausable_plugin_enabled(&sdk) && config.paused(&sdk) {
        panic!("permission denied: token paused")
    }
    let Ok(to) = Address::try_from(&input[..ADDRESS_LEN_BYTES]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let zero_address = Address::ZERO;
    assert_ne!(to, zero_address, "invalid recipient");
    let Some(amount) =
        U256::try_from_be_slice(&input[ADDRESS_LEN_BYTES..ADDRESS_LEN_BYTES + U256_LEN_BYTES])
    else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let total_supply = Settings::total_supply_get(&sdk);
    let (total_supply, overflow) = total_supply.overflowing_add(amount);
    if overflow {
        panic!("total supply overflow")
    }
    Settings::total_supply_set(&mut sdk, total_supply);
    Balance::add(&mut sdk, to, amount);
    emit_transfer_event(&mut sdk, &zero_address, &to, &amount);
    sdk.write(&fixed_bytes_from_u256(&U256::from(1)))
}

fn pause(mut sdk: impl SharedAPI, _input: &[u8]) {
    let mut config = Config::new();
    assert!(
        config.pausable_plugin_enabled(&sdk),
        "pausability not enabled"
    );
    let pauser = sdk.context().contract_caller();
    assert_eq!(
        pauser,
        Settings::pauser_get(&sdk),
        "permission denied: incorrect pauser address"
    );
    if config.paused(&sdk) {
        panic!("already paused");
    }
    config.pause(&sdk);
    config.save_flags(&mut sdk);
    emit_pause_event(&mut sdk, &pauser);
    sdk.write(&fixed_bytes_from_u256(&U256::from(1)));
}

fn unpause(mut sdk: impl SharedAPI, _input: &[u8]) {
    let mut config = Config::new();
    assert!(
        config.pausable_plugin_enabled(&sdk),
        "pausability not enabled"
    );
    let pauser = sdk.context().contract_caller();
    assert_eq!(
        pauser,
        Settings::pauser_get(&sdk),
        "permission denied: incorrect pauser address"
    );
    if !config.paused(&sdk) {
        panic!("already unpaused")
    }
    config.unpause(&sdk);
    config.save_flags(&mut sdk);
    emit_unpause_event(&mut sdk, &pauser);
    sdk.write(&fixed_bytes_from_u256(&U256::from(1)));
}

pub fn main_entry(mut sdk: impl SharedAPI) {
    let input_size = sdk.input_size();
    if input_size < SIG_LEN_BYTES as u32 {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    }
    let (sig, input) = sdk.input().split_at(SIG_LEN_BYTES);
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
        SIG_MINT => mint(sdk, input),
        SIG_PAUSE => pause(sdk, input),
        SIG_UNPAUSE => unpause(sdk, input),
        _ => {
            sdk.evm_exit(ERR_MALFORMED_INPUT);
        }
    }
}

entrypoint!(main_entry, deploy_entry);

#[cfg(test)]
mod tests {
    use crate::{
        storage::U256_LEN_BYTES,
        ERR_INSUFFICIENT_BALANCE,
        ERR_MALFORMED_INPUT,
        SIG_ALLOWANCE,
        SIG_APPROVE,
        SIG_BALANCE_OF,
        SIG_DECIMALS,
        SIG_MINT,
        SIG_NAME,
        SIG_PAUSE,
        SIG_SYMBOL,
        SIG_TOTAL_SUPPLY,
        SIG_TRANSFER,
        SIG_TRANSFER_FROM,
        SIG_UNPAUSE,
    };
    use fluentbase_sdk::{HashSet, U256};

    #[test]
    fn u256_bytes_size() {
        assert_eq!(size_of::<U256>(), U256_LEN_BYTES);
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
            SIG_MINT,
            SIG_PAUSE,
            SIG_UNPAUSE,
        ];
        let values_hashset: HashSet<u32> = values.into();
        assert_eq!(values.len(), values_hashset.len());
    }
}
