use crate::types::Bytes32;

pub trait LowLevelAPI {
    fn sys_read(target: &mut [u8], offset: u32);
    fn sys_input_size() -> u32;
    fn sys_write(value: &[u8]);
    fn sys_halt(exit_code: i32);
    fn sys_state() -> u32;

    fn crypto_keccak256(data: &[u8], output: &mut [u8]);
    fn crypto_poseidon(fr32_data: &[u8], output: &mut [u8]);
    fn crypto_poseidon2(
        fa32_data: &Bytes32,
        fb32_data: &Bytes32,
        fd32_data: &Bytes32,
        output32: &mut [u8],
    ) -> bool;
    fn crypto_ecrecover(digest: &[u8], sig: &[u8], output: &mut [u8], rec_id: u8);

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

    fn zktrie_open(root: &Bytes32);
    fn zktrie_update(key: &Bytes32, flags: u32, values: &[Bytes32]);
    fn zktrie_field(key: &Bytes32, field: u32, output: &mut [Bytes32]);
    fn zktrie_root(output: &mut Bytes32);
    fn zktrie_rollback();
    fn zktrie_commit();
    // fn zktrie_store(key: &Bytes32, val: &Bytes32);
    // fn zktrie_load(key: &Bytes32, val: &mut Bytes32);
}
