use crate::types::Bytes32;

pub trait LowLevelAPI {
    fn crypto_keccak256(data_offset: *const u8, data_len: u32, output32_offset: *mut u8);
    fn crypto_poseidon(data_offset: *const u8, data_len: u32, output32_offset: *mut u8);
    fn crypto_poseidon2(
        fa32_data: &Bytes32,
        fb32_data: &Bytes32,
        fd32_data: &Bytes32,
        output32: &mut [u8],
    ) -> bool;
    fn crypto_ecrecover(digest: &[u8], sig: &[u8], output: &mut [u8], rec_id: u8);

    fn sys_read(target: &mut [u8], offset: u32);
    fn sys_input_size() -> u32;
    fn sys_write(value: &[u8]);
    fn sys_halt(exit_code: i32);
    fn sys_output_size() -> u32;
    fn sys_read_output(target: *mut u8, offset: u32, length: u32);
    fn sys_state() -> u32;
    fn sys_exec(
        code_offset: *const u8,
        code_len: u32,
        input_offset: *const u8,
        input_len: u32,
        return_offset: *mut u8,
        return_len: u32,
        fuel_offset: *mut u32,
        state: u32,
    ) -> i32;

    fn jzkt_open(root32_ptr: *const u8);
    fn jzkt_checkpoint() -> (u32, u32);
    fn jzkt_get(key32_offset: *const u8, field: u32, output32_offset: *mut u8) -> bool;
    fn jzkt_update(key32_ptr: *const u8, flags: u32, vals32_ptr: *const [u8; 32], vals32_len: u32);
    fn jzkt_update_preimage(
        key32_ptr: *const u8,
        field: u32,
        preimage_ptr: *const u8,
        preimage_len: u32,
    ) -> bool;
    fn jzkt_remove(key32_offset: *const u8);
    fn jzkt_compute_root(output32_offset: *mut u8);
    fn jzkt_emit_log(
        key32_ptr: *const u8,
        topics32s_ptr: *const [u8; 32],
        topics32s_len: u32,
        data_ptr: *const u8,
        data_len: u32,
    );
    fn jzkt_commit(root32_offset: *mut u8);
    fn jzkt_rollback(checkpoint0: u32, checkpoint1: u32);
    fn jzkt_store(slot32_ptr: *const u8, value32_ptr: *const u8);
    fn jzkt_load(slot32_ptr: *const u8, value32_ptr: *mut u8) -> i32;
    fn jzkt_preimage_size(hash32_ptr: *const u8) -> u32;
    fn jzkt_preimage_copy(hash32_ptr: *const u8, preimage_ptr: *mut u8);

    fn rwasm_compile(input: &[u8], output: &mut [u8]) -> i32;
    fn rwasm_transact(
        address: &[u8],
        value: &[u8],
        input: &[u8],
        output: &mut [u8],
        fuel: u32,
        is_delegate: bool,
        is_static: bool,
    ) -> i32;
    fn rwasm_create(
        value32: &[u8],
        input_bytecode: &[u8],
        salt32: &[u8],
        deployed_contract_address20_output: &mut [u8],
        is_create2: bool,
    ) -> i32;

    fn statedb_get_code(key: &[u8], output: &mut [u8], code_offset: u32);
    fn statedb_get_code_size(key: &[u8]) -> u32;
    fn statedb_get_code_hash(key: &[u8], out_hash32: &mut [u8]) -> ();
    fn statedb_get_balance(address20: &[u8], out_balance32: &mut [u8], is_self: bool) -> ();
    fn statedb_set_code(key: &[u8], code: &[u8]);
    fn statedb_get_storage(key: &[u8], value: &mut [u8]);
    fn statedb_update_storage(key: &[u8], value: &[u8]);
    fn statedb_emit_log(topics: &[Bytes32], data: &[u8]);
}
