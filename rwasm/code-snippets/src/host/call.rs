use crate::{
    common::u256_be_to_u64tuple_le,
    common_sp::{stack_pop_u256, SP_BASE_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
};
use core::slice;
use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};

#[no_mangle]
pub fn host_call() {
    let gas = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let address = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let value = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
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

    <LowLevelSDK as LowLevelAPI>::rwasm_transact(
        &address[U256_BYTES_COUNT as usize - 12..],
        &value,
        input,
        output,
        fuel,
        false,
    );
}
