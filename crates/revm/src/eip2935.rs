use alloy_primitives::{Address, U256};
use fluentbase_sdk::{EIP2935_HISTORY_SERVE_WINDOW, SYSTEM_ADDRESS};
use std::{vec, vec::Vec};

pub(crate) fn eip2935_compute_storage_keys(
    input: &[u8],
    caller: &Address,
    block_number: u64,
) -> Option<Vec<U256>> {
    if input.len() != U256::BYTES {
        return None;
    } else if block_number == 0 {
        return None;
    }
    if caller == &SYSTEM_ADDRESS {
        let storage_slot = U256::from((block_number - 1) % EIP2935_HISTORY_SERVE_WINDOW);
        Some(vec![storage_slot])
    } else {
        let user_requested_block_number = U256::from_be_slice(&input);
        let block_number = U256::from(block_number);
        let block_number_prev = block_number - U256::from(1);
        if user_requested_block_number > block_number_prev {
            return None;
        } else if block_number - user_requested_block_number
            > U256::from(EIP2935_HISTORY_SERVE_WINDOW)
        {
            return None;
        }
        let storage_key = user_requested_block_number % U256::from(EIP2935_HISTORY_SERVE_WINDOW);
        Some(vec![storage_key])
    }
}
