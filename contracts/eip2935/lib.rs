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
    system_entrypoint, ContextReader, ExitCode, SystemAPI, EIP2935_HISTORY_SERVE_WINDOW,
    FUEL_DENOM_RATE, SYSTEM_ADDRESS, U256,
};

/// ------------------------------
/// Gas accounting (Prague fork)
/// ------------------------------
/// These are "entry-to-throw" gas totals for the *read* path reverts,
/// derived from the exact opcode sequences in the EVM snippet.
///
/// ; --- prefix gate: if caller == 0xffff..fffe jump to write-path @0x46 ---
/// caller                               ; 2
/// push20 0xfffffffffffffffffffffffffffffffffffffffe ; 3
/// eq                                   ; 3
/// push1 0x46                            ; 3
/// jumpi                                 ; 10
/// ; prefix subtotal = 21
/// ; --- read-path length gate: if (calldatasize - 0x20) != 0 then revert @0x42 ---
/// push1 0x20                            ; 3
/// calldatasize                          ; 2
/// sub                                   ; 3
/// push1 0x42                            ; 3
/// jumpi                                 ; 10
/// ; length-check subtotal = 21
/// ; --- load arg (slot selector / block number) from calldata[0:32] ---
/// push0                                 ; 2
/// calldataload                           ; 3
/// ; --- future-block gate: if arg > (number - 1) then revert @0x42 ---
/// push1 0x01                            ; 3
/// number                                 ; 2
/// sub                                    ; 3
/// dup2                                   ; 3
/// gt                                     ; 3
/// push1 0x42                             ; 3
/// jumpi                                  ; 10
/// ; future-check subtotal = 32
/// ; --- too-old gate: if (number - arg) > 0x1fff then revert @0x42 ---
/// push2 0x1fff                           ; 3
/// dup2                                   ; 3
/// number                                 ; 2
/// sub                                    ; 3
/// gt                                     ; 3
/// push1 0x42                             ; 3
/// jumpi                                  ; 10
/// ; too-old subtotal = 27
/// ; --- read: sload[(arg mod 0x1fff)] and return it as 32 bytes ---
/// push2 0x1fff                           ; 3
/// swap1                                  ; 3
/// mod                                    ; 5
/// sload                                  ; syscall
/// push0                                  ; 2
/// mstore                                 ; 3 + memory expansion (3)
/// push1 0x20                             ; 3
/// push0                                  ; 2
/// return                                 ; 0  + memory expansion (0)
/// ; --- revert handler @0x42 ---
/// jumpdest                               ; 1
/// push0                                  ; 2
/// push0                                  ; 2
/// revert                                 ; 0
/// ; throw-tail subtotal = 5
/// ; --- write-path @0x46: store (arg mod (number-1) mod 0x1fff) into s[ (number-1) mod 0x1fff ] ---
/// jumpdest                               ; 1
/// push0                                  ; 2
/// calldataload                           ; 3
/// push2 0x1fff                           ; 3
/// push1 0x01                             ; 3
/// number                                 ; 2
/// sub                                    ; 3
/// mod                                    ; 5
/// sstore                                 ; syscall
/// stop                                   ; 0
///
/// GAS_BAD_BLOCK_INPUT_BRANCH = 21 + 21 + 5 = 47
/// GAS_INVALID_BLOCK_BRANCH = 21 + 21 + 32 + 5 = 79
/// GAS_BLOCK_TOO_OLD_BRANCH = 21 + 21 + 32 + 27 + 5 = 106
/// GAS_RETRIEVE_SUCCESS_BRANCH = 21 + 21 + 32 + 27 + 24 = 125
/// GAS_SUBMIT_SUCCESS_BRANCH = 21 + 22 = 43
const GAS_BAD_BLOCK_INPUT_BRANCH: u64 = 47;
const GAS_INVALID_BLOCK_BRANCH: u64 = 79;
const GAS_BLOCK_TOO_OLD_BRANCH: u64 = 106;
const GAS_RETRIEVE_SUCCESS_BRANCH: u64 = 125;
const GAS_SUBMIT_SUCCESS_BRANCH: u64 = 43;

#[inline(always)]
fn charge_and_panic<SDK: SystemAPI, T>(sdk: &mut SDK, gas: u64) -> Result<T, ExitCode> {
    sdk.charge_fuel(gas * FUEL_DENOM_RATE);
    Err(ExitCode::Panic)
}

/// Submit a path (SYSTEM_ADDRESS) — store block hash at slot (number-1) % EIP2935_HISTORY_SERVE_WINDOW.
///
/// Your EVM contract never reverts to `submit:`; it just sstores and stops.
/// In Rust, we still defend against malformed input and revert with the "len" cost
/// (closest to how the EVM read path throws on bad calldata length).
fn submit<SDK: SystemAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
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
    sdk.charge_fuel(GAS_SUBMIT_SUCCESS_BRANCH * FUEL_DENOM_RATE);
    Ok(())
}

/// Read path — validates calldata, range checks, then returns hash from ring buffer.
fn retrieve<SDK: SystemAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
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
    sdk.charge_fuel(GAS_RETRIEVE_SUCCESS_BRANCH * FUEL_DENOM_RATE);
    let hash = result.data;
    sdk.write(hash.to_be_bytes::<{ U256::BYTES }>());
    Ok(())
}

pub fn entrypoint<SDK: SystemAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    let caller = sdk.context().contract_caller();
    if caller == SYSTEM_ADDRESS {
        submit(sdk)
    } else {
        retrieve(sdk)
    }
}

system_entrypoint!(entrypoint);
