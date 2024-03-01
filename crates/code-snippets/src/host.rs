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
    consts::{GAS_LIMIT_HARDCODED, U256_BYTES_COUNT},
};
use core::slice;
use fluentbase_core::ExitCode;
use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};

#[cfg(feature = "host_balance")]
mod balance;
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
#[cfg(feature = "host_extcodecopy")]
mod extcodecopy;
#[cfg(feature = "host_extcodehash")]
mod extcodehash;
#[cfg(feature = "host_extcodesize")]
mod extcodesize;
#[cfg(feature = "host_gaslimit")]
mod gaslimit;
#[cfg(feature = "host_log0")]
mod log0;
#[cfg(feature = "host_log1")]
mod log1;
#[cfg(feature = "host_log2")]
mod log2;
#[cfg(feature = "host_log3")]
mod log3;
#[cfg(feature = "host_log4")]
mod log4;
#[cfg(feature = "host_number")]
mod number;
#[cfg(feature = "host_selfbalance")]
mod selfbalance;
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

#[deprecated]
#[inline]
pub fn host_call_impl<const IS_DELEGATE: bool, const IS_STATIC: bool>() {
    let gas = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let address = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let value = if IS_STATIC || IS_DELEGATE {
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
        IS_DELEGATE,
        IS_STATIC,
    );
    if exit_code == 0 {
        stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, u256_one());
    } else {
        stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, u256_zero());
    }
}

#[inline]
pub fn host_call_impl_v2<const IS_DELEGATE: bool, const IS_STATIC: bool>() {
    let gas = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let address = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let value = if IS_STATIC || IS_DELEGATE {
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

    let exit_code = fluentbase_core::evm::call::_evm_call(
        fuel,
        address[U256_BYTES_COUNT as usize - 20..].as_ptr(),
        value.as_ptr(),
        args_offset.0 as *const u8,
        args_size.0 as u32,
        ret_offset.0 as *mut u8,
        ret_size.0 as u32,
    );
    if exit_code == ExitCode::Ok {
        stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, u256_one());
        return;
    }
    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, u256_zero());
}

#[deprecated]
#[inline]
pub fn host_create_impl<const IS_CREATE2: bool>() {
    let value = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let init_bytecode_offset = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let mut init_bytecode_size = if IS_CREATE2 {
        (0, stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT))
    } else {
        stack_peek_u256(SP_BASE_MEM_OFFSET_DEFAULT, 0)
    };
    let salt = if IS_CREATE2 {
        stack_peek_u256(SP_BASE_MEM_OFFSET_DEFAULT, 0)
    } else {
        (init_bytecode_size.0, [0u8; U256_BYTES_COUNT as usize])
    };
    let deployed_contract_address20_out_offset = if IS_CREATE2 {
        salt.0
    } else {
        init_bytecode_size.0
    };

    let init_bytecode_offset = u256_be_to_u64tuple_le(init_bytecode_offset);
    let init_bytecode_size = u256_be_to_u64tuple_le(init_bytecode_size.1);

    let init_bytecode = unsafe {
        slice::from_raw_parts(
            init_bytecode_offset.0 as *const u8,
            init_bytecode_size.0 as usize,
        )
    };
    let deployed_contract_address20_out =
        unsafe { slice::from_raw_parts_mut(deployed_contract_address20_out_offset as *mut u8, 20) };

    let exit_code = <LowLevelSDK as LowLevelAPI>::rwasm_create(
        &value,
        init_bytecode,
        salt.1.as_slice(),
        deployed_contract_address20_out,
        false,
    );
    if exit_code != 0 {
        stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, [0u8; U256_BYTES_COUNT as usize]);
        return;
    }
    stack_push_u256(
        SP_BASE_MEM_OFFSET_DEFAULT,
        u256_from_be_slice(deployed_contract_address20_out),
    );
}

#[inline]
pub fn host_create_impl_v2<const IS_CREATE2: bool>() {
    let value = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let init_bytecode_offset = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let mut init_bytecode_size = if IS_CREATE2 {
        (0, stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT))
    } else {
        stack_peek_u256(SP_BASE_MEM_OFFSET_DEFAULT, 0)
    };
    let salt = if IS_CREATE2 {
        stack_peek_u256(SP_BASE_MEM_OFFSET_DEFAULT, 0)
    } else {
        (init_bytecode_size.0, [0u8; U256_BYTES_COUNT as usize])
    };
    let deployed_contract_address20_out_offset = if IS_CREATE2 {
        salt.0
    } else {
        init_bytecode_size.0
    };

    let init_bytecode_offset = u256_be_to_u64tuple_le(init_bytecode_offset).0;
    let init_bytecode_size = u256_be_to_u64tuple_le(init_bytecode_size.1).0;

    let deployed_contract_address20_out =
        unsafe { slice::from_raw_parts_mut(deployed_contract_address20_out_offset as *mut u8, 20) };

    let exit_code = if IS_CREATE2 {
        fluentbase_core::evm::create2::_evm_create2(
            value.as_ptr(),
            init_bytecode_offset as *const u8,
            init_bytecode_size as u32,
            salt.0 as *const u8,
            deployed_contract_address20_out.as_mut_ptr(),
            GAS_LIMIT_HARDCODED,
        )
    } else {
        fluentbase_core::evm::create::_evm_create(
            value.as_ptr(),
            init_bytecode_offset as *const u8,
            init_bytecode_size as u32,
            deployed_contract_address20_out.as_mut_ptr(),
            GAS_LIMIT_HARDCODED,
        )
    };
    if exit_code != fluentbase_core::ExitCode::Ok {
        stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, [0u8; U256_BYTES_COUNT as usize]);
        return;
    }
    stack_push_u256(
        SP_BASE_MEM_OFFSET_DEFAULT,
        u256_from_be_slice(deployed_contract_address20_out),
    );
}

#[inline]
pub fn host_log<const TOPIC_COUNT: usize>() {
    assert!(TOPIC_COUNT <= 4);
    let offset = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let size = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let offset = u256_be_to_u64tuple_le(offset);
    let size = u256_be_to_u64tuple_le(size);

    // TODO process incorrect params

    let data = unsafe { slice::from_raw_parts(offset.0 as *const u8, size.0 as usize) };

    match TOPIC_COUNT {
        0 => {
            LowLevelSDK::statedb_emit_log(&[], data);
        }
        1 => LowLevelSDK::statedb_emit_log(&[stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT)], data),
        2 => LowLevelSDK::statedb_emit_log(
            &[
                stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT),
                stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT),
            ],
            data,
        ),
        3 => LowLevelSDK::statedb_emit_log(
            &[
                stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT),
                stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT),
                stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT),
            ],
            data,
        ),
        4 => LowLevelSDK::statedb_emit_log(
            &[
                stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT),
                stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT),
                stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT),
                stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT),
            ],
            data,
        ),
        _ => {}
    };
}
