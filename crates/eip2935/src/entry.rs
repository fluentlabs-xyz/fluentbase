use crate::{
    consts::HISTORY_SERVE_WINDOW,
    helpers::{slice_from_u256, u256_try_from_slice},
};
use fluentbase_helpers::consts::U256_LEN_BYTES;
use fluentbase_sdk::{ContextReader, SharedAPI, U256};
use fluentbase_types::SYSTEM_ADDRESS;

/// impl of https://eips.ethereum.org/EIPS/eip-2935

fn throw(sdk: &mut impl SharedAPI) -> ! {
    sdk.evm_exit(1);
}

fn submit(mut sdk: impl SharedAPI) {
    let input = sdk.input();
    if input.len() != U256_LEN_BYTES {
        throw(&mut sdk);
    }
    let hash_value = if let Some(v) = u256_try_from_slice(&input[..U256_LEN_BYTES]) {
        v
    } else {
        throw(&mut sdk);
    };
    let block_number = sdk.context().block_number();
    if block_number <= 0 {
        throw(&mut sdk);
    }
    let slot = (block_number as usize - 1) % HISTORY_SERVE_WINDOW;
    let hash_key = U256::from(slot);
    sdk.write_storage(hash_key, hash_value);
}
fn retrieve(mut sdk: impl SharedAPI) {
    let input = sdk.input();
    if input.len() != U256_LEN_BYTES {
        throw(&mut sdk);
    }
    // ;; Check if the input is requesting a block hash before the earliest available
    // ;; hash currently. Since we've verified that input <= number - 1, we know
    // ;; there will be no overflow during the subtraction of number - input.
    let user_requested_block_number = u256_try_from_slice(&input[..U256_LEN_BYTES]).unwrap();
    let block_number = sdk.context().block_number();
    if block_number <= 0 {
        throw(&mut sdk);
    }
    let block_number = U256::from(block_number);
    let block_number_prev = block_number - U256::from(1);
    if user_requested_block_number > block_number_prev {
        throw(&mut sdk);
    }
    if block_number - user_requested_block_number > U256::from(HISTORY_SERVE_WINDOW) {
        throw(&mut sdk);
    }
    // ;; Load the hash.
    let hash_key = user_requested_block_number % U256::from(HISTORY_SERVE_WINDOW);
    let hash_value = sdk.storage(&hash_key);
    sdk.write(slice_from_u256(&hash_value));
}

pub fn main_entry(mut sdk: impl SharedAPI) {
    let caller = sdk.context().contract_caller();
    if caller == SYSTEM_ADDRESS {
        submit(sdk);
        return;
    }
    retrieve(sdk);
}
