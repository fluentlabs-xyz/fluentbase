//! A contract that implements EIP-2935 (https://eips.ethereum.org/EIPS/eip-2935)
#![cfg_attr(target_arch = "wasm32", no_std, no_main)]

#[cfg(test)]
mod tests;

use fluentbase_sdk::{
    system_entrypoint2, ContextReader, ExitCode, SharedAPI, EIP2935_HISTORY_SERVE_WINDOW,
    SYSTEM_ADDRESS, U256,
};

fn submit<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    let input = sdk.input();
    if input.len() != U256::BYTES {
        return Err(ExitCode::Panic);
    }
    let Some(hash_value) = U256::try_from_be_slice(&input[..U256::BYTES]) else {
        return Err(ExitCode::Panic);
    };
    let block_number = sdk.context().block_number();
    if block_number <= 0 {
        return Err(ExitCode::Panic);
    }
    let slot = (block_number - 1) % EIP2935_HISTORY_SERVE_WINDOW;
    let hash_key = U256::from(slot);
    sdk.write_storage(hash_key, hash_value).ok()?;
    Ok(())
}

fn retrieve<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    let input = sdk.input();
    if input.len() != U256::BYTES {
        return Err(ExitCode::Panic);
    }
    // Check if the input is requesting a block hash before the earliest available hash currently.
    // Since we've verified that input <= number - 1, we know there will be no overflow during the
    // subtraction of number - input.
    let Some(user_requested_block_number) = U256::try_from_be_slice(&input[..U256::BYTES]) else {
        return Err(ExitCode::Panic);
    };
    let block_number = sdk.context().block_number();
    if block_number <= 0 {
        return Err(ExitCode::Panic);
    }
    let block_number = U256::from(block_number);
    let block_number_prev = block_number - U256::from(1);
    if user_requested_block_number > block_number_prev {
        return Err(ExitCode::Panic);
    }
    if block_number - user_requested_block_number > U256::from(EIP2935_HISTORY_SERVE_WINDOW) {
        return Err(ExitCode::Panic);
    }
    // Load the hash.
    let hash_key = user_requested_block_number % U256::from(EIP2935_HISTORY_SERVE_WINDOW);
    let hash_value = sdk.storage(&hash_key).ok()?;
    sdk.write(hash_value.to_be_bytes::<{ U256::BYTES }>());
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

system_entrypoint2!(entrypoint);
