use crate::{HALT_CODE_EXIT, HALT_CODE_PANIC};

extern "C" {
    fn _sys_halt(code: u32);
    fn _sys_read(target: *mut u8, offset: u32, length: u32);
    fn _sys_write(offset: u32, length: u32);
    fn _evm_stop();
    fn _evm_return(offset: *const u8, size: u32);
    fn _evm_keccak256(offset: *const u8, size: u32, dest: *mut u8);
    fn _evm_address(dest: *mut u8); // no
    fn _evm_balance(address: *const u8, dest: *mut u8); // no
    fn _evm_origin(dest: *mut u8); // no
    fn _evm_caller(dest: *mut u8); // no
    fn _evm_callvalue(dest: *mut u8);
    fn _evm_calldataload(offset: u32, dest: *mut u8);
    fn _evm_calldatasize(dest: *mut u32);
    fn _evm_calldatacopy(mem_offset: *mut u8, data_offset: *const u8, length: u32);
    fn _evm_codesize(dest: *mut u32);
    fn _evm_codecopy(mem_offset: *mut u8, code_offset: *const u8, length: u32);
    fn _evm_gasprice(dest: *mut u8); // no
    fn _evm_extcodesize(address: *const u8, dest: *mut u32); // no
    fn _evm_extcodecopy(address: *const u8, mem_offset: *const u8, code_offset: u32, length: u32); // no
    fn _evm_extcodehash(address: *const u8, dest: *mut u8); // no
    fn _evm_returndatasize(dest: *mut u32);
    fn _evm_returndatacopy(mem_offset: *const u8, data_offset: u32, length: u32);
    fn _evm_blockhash(num: u64, dest: *mut u8); // no
    fn _evm_coinbase(dest: *mut u8); // no
    fn _evm_timestamp(dest: *mut i64);
    fn _evm_number(dest: *mut u64); // no
    fn _evm_difficulty(dest: *mut u8); // no
    fn _evm_gaslimit(dest: *mut u64); // no
    fn _evm_chainid(dest: *mut u8); // no
    fn _evm_basefee(dest: *mut u8); // no
    fn _evm_sload(slot: *const u8, dest: *mut u8); // no
    fn _evm_sstore(slot: *const u8, value: *const u8); // no
    fn _evm_log0(data_offset: i32, data_length: u32);
    fn _evm_log1(data_offset: i32, data_length: u32, topic0: *const u8);
    fn _evm_log2(data_offset: i32, data_length: u32, topic0: *const u8, topic1: *const u8);
    fn _evm_log3(
        data_offset: i32,
        data_length: u32,
        topic0: *const u8,
        topic1: *const u8,
        topic2: *const u8,
    );
    fn _evm_log4(
        data_offset: i32,
        data_length: u32,
        topic0: *const u8,
        topic1: *const u8,
        topic2: *const u8,
        topic3: *const u8,
    );
    fn _evm_create(value: *const u8, bytecode_offset: *const u8, bytecode_length: u32); // no
    fn _evm_call(
        gas: u64,
        address: *const u8,
        value: *const u8,
        input_offset: *const u8,
        input_length: u32,
        return_offset: *const u8,
        return_length: u32,
        dest: *mut bool,
    ); // no
    fn _evm_callcode(
        gas: u64,
        address: *const u8,
        value: *const u8,
        input_offset: *const u8,
        input_length: u32,
        return_offset: *const u8,
        return_length: u32,
        dest: *mut bool,
    ); // no
    fn _evm_delegatecall(
        gas: u64,
        address: *const u8,
        input_offset: *const u8,
        input_length: u32,
        return_offset: *const u8,
        return_length: u32,
        dest: *mut bool,
    ); // no
    fn _evm_create2(
        value: *const u8,
        bytecode_offset: *const u8,
        bytecode_length: u32,
        salt: *const u8,
        dest: *mut u8,
    ); // no
    fn _evm_staticcall(
        gas: u64,
        address: *const u8,
        input_offset: *const u8,
        input_length: u32,
        return_offset: *const u8,
        return_length: u32,
        dest: *mut bool,
    ); // no
    fn _evm_revert(error_offset: *const u8, error_length: u32); // no
    fn _evm_selfdestruct(beneficiary: *const u8); // no
                                                  // forward_call!(res, "env", "zktrie_open", fn zktrie_open(root_start_offset: i32, root_len:
                                                  // i32, key_start_offset: i32, key_len: i32, leafs_start_offset: i32, leafs_count: i32) -> ());
    fn zktrie_open(
        root_start_offset: i32,
        root_len: i32,
        keys_offset: i32,
        leafs_offset: i32,
        accounts_count: i32,
    ); // no
       //         // account updates
       //         forward_call!(res, "env", "zktrie_update_nonce", fn zktrie_update_nonce(offset: i32,
       // length: i32) -> ());         forward_call!(res, "env", "zktrie_update_balance", fn
       // zktrie_update_balance(offset: i32, length: i32) -> ());         forward_call!(res,
       // "env", "zktrie_update_storage_root", fn zktrie_update_storage_root(offset: i32, length: i32)
       // -> ());         forward_call!(res, "env", "zktrie_update_code_hash", fn
       // zktrie_update_code_hash(offset: i32, length: i32) -> ());         forward_call!(res,
       // "env", "zktrie_update_code_size", fn zktrie_update_code_size(offset: i32, length: i32) ->
       // ());         // account gets
       //         forward_call!(res, "env", "zktrie_get_nonce", fn zktrie_get_nonce(key_offset: i32,
       // output_offset: i32) -> ());         forward_call!(res, "env", "zktrie_get_balance", fn
       // zktrie_get_balance(key_offset: i32, output_offset: i32) -> ());         forward_call!
       // (res, "env", "zktrie_get_storage_root", fn zktrie_get_storage_root(key_offset: i32,
       // output_offset: i32) -> ());         forward_call!(res, "env", "zktrie_get_code_hash",
       // fn zktrie_get_code_hash(key_offset: i32, output_offset: i32) -> ());
       //         forward_call!(res, "env", "zktrie_get_code_size", fn zktrie_get_code_size(key_offset:
       // i32, output_offset: i32) -> ());         // store updates
       //         forward_call!(res, "env", "zktrie_update_store", fn zktrie_update_store(offset: i32,
       // length: i32) -> ());         // store gets
       //         forward_call!(res, "env", "zktrie_get_store", fn zktrie_get_store(key_offset: i32,
       // output_offset: i32) -> ());
}

#[inline(always)]
pub fn sys_read(target: *mut u8, offset: u32, len: u32) {
    unsafe { _sys_read(target, offset, len) }
}

#[inline(always)]
pub fn sys_write(offset: u32, len: u32) {
    unsafe { _sys_write(offset, len) }
}

#[inline(always)]
pub fn sys_read_slice(target: &mut [u8], offset: u32) {
    unsafe { _sys_read(target.as_mut_ptr(), offset, target.len() as u32) }
}

#[inline(always)]
pub fn evm_return_raw(ptr: *const u8, size: u32) {
    unsafe { _evm_return(ptr, size) }
}

#[inline(always)]
pub fn evm_return_slice(return_data: &[u8]) {
    unsafe { _evm_return(return_data.as_ptr(), return_data.len() as u32) }
}

#[inline(always)]
pub fn sys_exit() {
    unsafe { _sys_halt(HALT_CODE_EXIT) }
}

#[inline(always)]
pub fn sys_panic() {
    unsafe { _sys_halt(HALT_CODE_PANIC) }
}

#[inline(always)]
pub fn zktrie_open_(
    root_start_offset: i32,
    root_len: i32,
    keys_offset: i32,
    leafs_offset: i32,
    accounts_count: i32,
) {
    unsafe {
        zktrie_open(
            root_start_offset,
            root_len,
            keys_offset,
            leafs_offset,
            accounts_count,
        )
    }
}
