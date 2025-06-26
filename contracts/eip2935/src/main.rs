#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate core;

use fluentbase_sdk::{entrypoint, ContextReader, ExitCode, SharedAPI, SYSTEM_ADDRESS, U256};

/// impl of https://eips.ethereum.org/EIPS/eip-2935

pub const BLOCKHASH_SERVE_WINDOW: usize = 256;
pub const HISTORY_SERVE_WINDOW: usize = 8191;

fn submit(mut sdk: impl SharedAPI) {
    let input = sdk.input();
    if input.len() != U256::BYTES {
        sdk.exit(ExitCode::Panic);
    }
    let hash_value = U256::try_from_le_slice(&input[..U256::BYTES]).unwrap_or_else(|| {
        sdk.exit(ExitCode::Panic);
    });
    let block_number = sdk.context().block_number();
    if block_number <= 0 {
        sdk.exit(ExitCode::Panic);
    }
    let slot = (block_number as usize - 1) % HISTORY_SERVE_WINDOW;
    let hash_key = U256::from(slot);
    sdk.write_storage(hash_key, hash_value);
}

fn retrieve(mut sdk: impl SharedAPI) {
    let input = sdk.input();
    if input.len() != U256::BYTES {
        sdk.exit(ExitCode::Panic);
    }
    // Check if the input is requesting a block hash before the earliest available hash currently.
    // Since we've verified that input <= number - 1, we know there will be no overflow during the
    // subtraction of number - input.
    let user_requested_block_number = U256::try_from_le_slice(&input[..U256::BYTES]).unwrap();
    let block_number = sdk.context().block_number();
    if block_number <= 0 {
        sdk.exit(ExitCode::Panic);
    }
    let block_number = U256::from(block_number);
    let block_number_prev = block_number - U256::from(1);
    if user_requested_block_number > block_number_prev {
        sdk.exit(ExitCode::Panic);
    }
    if block_number - user_requested_block_number > U256::from(HISTORY_SERVE_WINDOW) {
        sdk.exit(ExitCode::Panic);
    }
    // Load the hash.
    let hash_key = user_requested_block_number % U256::from(HISTORY_SERVE_WINDOW);
    let hash_value = sdk.storage(&hash_key);
    sdk.write(hash_value.as_le_slice());
}

pub fn entrypoint(sdk: impl SharedAPI) {
    let caller = sdk.context().contract_caller();
    if caller == SYSTEM_ADDRESS {
        submit(sdk);
    } else {
        retrieve(sdk);
    }
}

entrypoint!(entrypoint);
