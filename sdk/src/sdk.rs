use crate::types::Bytes32;

pub trait LowLevelCryptoSDK {
    fn crypto_keccak256(data: &[u8], output: &mut [u8]);
    fn crypto_poseidon(fr32_data: &[u8], output: &mut [u8]);
    fn crypto_poseidon2(
        fa32_data: &Bytes32,
        fb32_data: &Bytes32,
        fd32_data: &Bytes32,
        output32: &mut [u8],
    ) -> bool;
    fn crypto_ecrecover(digest: &[u8], sig: &[u8], output: &mut [u8], rec_id: u8);
}

pub trait LowLevelSysSDK {
    fn sys_read(target: &mut [u8], offset: u32);
    fn sys_input_size() -> u32;
    fn sys_write(value: &[u8]);
    fn sys_halt(exit_code: i32);
    fn sys_state() -> u32;
}

pub trait LowLevelRwasmSDK {
    fn rwasm_compile(input: &[u8], output: &mut [u8]) -> i32;
    fn rwasm_transact(
        bytecode: &[u8],
        input: &[u8],
        output: &mut [u8],
        state: u32,
        fuel_limit: u32,
    ) -> i32;
}

pub trait LowLevelZkTrieSDK {
    fn zktrie_open(root: &Bytes32) -> u32;
    fn zktrie_update(trie: u32, key: &Bytes32, flags: u32, values: &[Bytes32]);
    fn zktrie_get(trie: u32, key: &Bytes32, output: &mut [Bytes32]);
    fn zktrie_root(trie: u32, output: &mut Bytes32);
}
