use crate::{
    common::{u256_be_to_u64tuple_le, u256_from_be_slice},
    common_sp::{
        stack_peek_u256,
        stack_pop_u256,
        stack_push_u256,
        u256_one,
        u256_zero,
        SP_BASE_MEM_OFFSET_DEFAULT,
    },
    consts::U256_BYTES_COUNT,
};
use core::slice;
use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};

#[cfg(feature = "host_basefee")]
mod basefee;
#[cfg(feature = "host_blockhash")]
mod blockhash;
#[cfg(feature = "host_call")]
mod call;
#[cfg(feature = "host_chainid")]
mod chainid;
#[cfg(feature = "host_coinbase")]
mod coinbase;
#[cfg(feature = "host_create")]
mod create;
#[cfg(feature = "host_create2")]
mod create2;
#[cfg(feature = "host_delegatecall")]
mod delegatecall;
#[cfg(feature = "host_gaslimit")]
mod gaslimit;
#[cfg(feature = "host_number")]
mod number;
#[cfg(feature = "host_sload")]
mod sload;
#[cfg(feature = "host_sstore")]
mod sstore;
#[cfg(feature = "host_staticcall")]
mod staticcall;
#[cfg(feature = "host_timestamp")]
mod timestamp;
#[cfg(feature = "host_tload")]
mod tload;
#[cfg(feature = "host_tstore")]
mod tstore;

#[inline]
pub fn host_call_impl(is_delegate: bool, is_static: bool) {
    let gas = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let address = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let value = if is_static || is_delegate {
        u256_zero()
    } else {
        stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT)
    };
    let args_offset = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let args_size = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let ret_offset = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let ret_size = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let gas = u256_be_to_u64tuple_le(gas);
    let fuel: u32 = {
        if gas.0 > u32::MAX as u64 || gas.1 > 0 || gas.2 > 0 || gas.3 > 0 {
            u32::MAX
        } else {
            gas.0 as u32
        }
    };
    let args_offset = u256_be_to_u64tuple_le(args_offset);
    let args_size = u256_be_to_u64tuple_le(args_size);
    let ret_offset = u256_be_to_u64tuple_le(ret_offset);
    let ret_size = u256_be_to_u64tuple_le(ret_size);

    let input = unsafe { slice::from_raw_parts(args_offset.0 as *const u8, args_size.0 as usize) };
    let output = unsafe { slice::from_raw_parts_mut(ret_offset.0 as *mut u8, ret_size.0 as usize) };

    let exit_code = <LowLevelSDK as LowLevelAPI>::rwasm_transact(
        &address[U256_BYTES_COUNT as usize - 20..],
        &value,
        input,
        output,
        fuel,
        is_delegate,
        is_static,
    );
    if exit_code == 0 {
        stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, u256_one());
    } else {
        stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, u256_zero());
    }
}

#[inline]
pub fn host_create_impl(is_create2: bool) {
    let value = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let init_bytecode_offset = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let mut init_bytecode_size = if is_create2 {
        (0, stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT))
    } else {
        stack_peek_u256(SP_BASE_MEM_OFFSET_DEFAULT, 0)
    };
    let salt = if is_create2 {
        stack_peek_u256(SP_BASE_MEM_OFFSET_DEFAULT, 0)
    } else {
        (init_bytecode_size.0, [0u8; U256_BYTES_COUNT as usize])
    };
    let deployed_contract_address20_offset = if is_create2 {
        salt.0
    } else {
        init_bytecode_size.0
    };

    let init_bytecode_offset = u256_be_to_u64tuple_le(init_bytecode_offset);
    let init_bytecode_size = u256_be_to_u64tuple_le(init_bytecode_size.1);

    let input = unsafe {
        slice::from_raw_parts(
            init_bytecode_offset.0 as *const u8,
            init_bytecode_size.0 as usize,
        )
    };
    let deployed_contract_address20 =
        unsafe { slice::from_raw_parts_mut(deployed_contract_address20_offset as *mut u8, 20) };

    let exit_code = <LowLevelSDK as LowLevelAPI>::rwasm_create(
        &value,
        input,
        salt.1.as_slice(),
        deployed_contract_address20,
        false,
    );
    if exit_code != 0 {
        stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, [0u8; U256_BYTES_COUNT as usize]);
        return;
    }
    stack_push_u256(
        SP_BASE_MEM_OFFSET_DEFAULT,
        u256_from_be_slice(deployed_contract_address20),
    );
}
