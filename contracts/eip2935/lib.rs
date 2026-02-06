//! EIP-2935 pre-deploy (ring buffer of recent block hashes).
//!
//! This version charges different gas amounts before each Panic/Revert
//! to match the *EVM assembly control-flow* you provided.
//!
//! Charges are expressed in EVM gas units then multiplied by FUEL_DENOM_RATE.
//!
//! https://eips.ethereum.org/EIPS/eip-2935

#![cfg_attr(target_arch = "wasm32", no_std, no_main)]

#[cfg(test)]
mod tests;

use fluentbase_sdk::{
    system_entrypoint, ContextReader, ExitCode, SharedAPI, EIP2935_HISTORY_SERVE_WINDOW,
    FUEL_DENOM_RATE, SYSTEM_ADDRESS, U256,
};

/// ------------------------------
/// Gas accounting (Prague fork)
/// ------------------------------
/// These are "entry-to-throw" gas totals for the *read* path reverts,
/// derived from the exact opcode sequences in the EVM snippet.
///
/// Throw tail in snippet:
///   JUMPDEST (1) + PUSH0 (3) + PUSH0 (3) + REVERT (0) = 7
///
/// Prefix gate (caller != SYSADDR; jump not taken):
///   CALLER(2) + PUSH20(3) + EQ(3) + JUMPI(10) = 18
///
/// Length check block:
///   PUSH1(3) + CALLDATASIZE(2) + SUB(3) + JUMPI(10) = 18
///
/// Future-block check block:
///   PUSH0(3) + CALLDATALOAD(3) + PUSH1(3) + NUMBER(2) + SUB(3)
///   + DUP2(3) + GT(3) + JUMPI(10) = 30
///
/// Too-old check block:
///   PUSH(3) + DUP2(3) + NUMBER(2) + SUB(3) + GT(3) + JUMPI(10) = 24
///
/// Totals:
/// - Bad length: 18 + 18 + 7 = 43
/// - Future block: 18 + 18 + 30 + 7 = 73
/// - Too old block: 18 + 18 + 30 + 24 + 7 = 97
const GAS_BAD_BLOCK_INPUT_BRANCH: u64 = 43;
const GAS_INVALID_BLOCK_BRANCH: u64 = 73;
const GAS_BLOCK_TOO_OLD_BRANCH: u64 = 97;
const GAS_RETRIEVE_SUCCESS_BRANCH: u64 = 113;
const GAS_SUBMIT_SUCCESS_BRANCH: u64 = 41;

#[inline(always)]
fn charge_and_panic<SDK: SharedAPI, T>(sdk: &mut SDK, gas: u64) -> Result<T, ExitCode> {
    sdk.charge_fuel(gas * FUEL_DENOM_RATE);
    Err(ExitCode::Panic)
}

/// Submit a path (SYSTEM_ADDRESS) — store block hash at slot (number-1) % EIP2935_HISTORY_SERVE_WINDOW.
///
/// Your EVM contract never reverts to `submit:`; it just sstores and stops.
/// In Rust, we still defend against malformed input and revert with the "len" cost
/// (closest to how the EVM read path throws on bad calldata length).
fn submit<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    // Make sure the input is correct
    let input_size = sdk.input_size();
    if input_size != U256::BYTES as u32 {
        return charge_and_panic(sdk, GAS_BAD_BLOCK_INPUT_BRANCH);
    }
    let mut hash_value = [0u8; 32];
    sdk.read(&mut hash_value, 0);
    let hash_value = U256::from_be_bytes(hash_value);

    // EVM would underflow if number=0, but on Ethereum bn>=0 always and genesis is 0.
    // We keep a guard; if triggered, treat it like a "future/invalid" style revert.
    let block_number = sdk.context().block_number();
    if block_number == 0 {
        return charge_and_panic(sdk, GAS_INVALID_BLOCK_BRANCH);
    }

    let storage_slot = U256::from((block_number - 1) % EIP2935_HISTORY_SERVE_WINDOW);
    let result = sdk.write_storage(storage_slot, hash_value);
    if !result.status.is_ok() {
        // Storage write here can't fail, even if it fails, it causes trap and charges all gas available
        return Err(result.status);
    }
    sdk.charge_fuel(result.fuel_consumed + GAS_SUBMIT_SUCCESS_BRANCH * FUEL_DENOM_RATE);
    Ok(())
}

/// Read path — validates calldata, range checks, then returns hash from ring buffer.
fn retrieve<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    // Make sure the input is correct
    let input_size = sdk.input_size();
    if input_size != U256::BYTES as u32 {
        return charge_and_panic(sdk, GAS_BAD_BLOCK_INPUT_BRANCH);
    }
    let mut requested_bn = [0u8; 32];
    sdk.read(&mut requested_bn, 0);
    let requested_bn = U256::from_be_bytes(requested_bn);

    let block_number = sdk.context().block_number();
    if block_number == 0 {
        // Matches the spirit of "requested > number-1" invalidity.
        return charge_and_panic(sdk, GAS_INVALID_BLOCK_BRANCH);
    }

    let block_number = U256::from(block_number);
    let block_number_prev = block_number - U256::from(1);

    // EVM: if input > number-1 => throw
    if requested_bn > block_number_prev {
        return charge_and_panic(sdk, GAS_INVALID_BLOCK_BRANCH);
    }
    // EVM: if (number - input) > EIP2935_HISTORY_SERVE_WINDOW => throw
    // Note: EVM proves input <= number-1 first, so subtraction can't underflow.
    let block_age = block_number - requested_bn;
    if block_age > U256::from(EIP2935_HISTORY_SERVE_WINDOW) {
        return charge_and_panic(sdk, GAS_BLOCK_TOO_OLD_BRANCH);
    }

    // EVM: slot = input % EIP2935_HISTORY_SERVE_WINDOW ; sload(slot)
    let slot = requested_bn % U256::from(EIP2935_HISTORY_SERVE_WINDOW);
    let result = sdk.storage(&slot);
    if !result.status.is_ok() {
        // Storage write here can't fail, even if it fails, it causes trap and charges all gas available
        return Err(result.status);
    }
    sdk.charge_fuel(result.fuel_consumed + GAS_RETRIEVE_SUCCESS_BRANCH * FUEL_DENOM_RATE);
    let hash = result.data;
    sdk.write(hash.to_be_bytes::<{ U256::BYTES }>());
    Ok(())
}

pub fn entrypoint<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    let caller = sdk.context().contract_caller();
    if caller == SYSTEM_ADDRESS {
        submit(sdk)
    } else {
        retrieve(sdk)
    }
}

system_entrypoint!(entrypoint);
