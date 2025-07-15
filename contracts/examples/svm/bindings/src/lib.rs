use core::mem::size_of;
use solana_pubkey::Pubkey;

extern "C" {
    fn sol_log_pubkey(pubkey: *const Pubkey);
}
pub fn log_pubkey_native(pubkey: &Pubkey) {
    unsafe { sol_log_pubkey(pubkey as *const Pubkey) }
}

extern "C" {
    fn sol_log_data(values: *const u8, values_len: u64);
}
pub fn log_data_native(data: &[&[u8]]) {
    unsafe { sol_log_data(data.as_ptr() as *const u8, data.len() as u64) }
}

extern "C" {
    fn sol_set_return_data(values: *const u8, values_len: u64);
}
pub fn set_return_data_native(data: &[u8]) {
    unsafe { sol_set_return_data(data.as_ptr(), data.len() as u64) }
}

extern "C" {
    fn sol_get_return_data(data: *const u8, data_len: u64, program_id: *const u8) -> u64;
}
pub fn get_return_data_native(data_buffer_len: usize) -> (Pubkey, Vec<u8>, u64) {
    let program_id = Pubkey::default();
    let data = vec![0u8; data_buffer_len];
    let len = unsafe {
        sol_get_return_data(
            data.as_ptr(),
            data.len() as u64,
            program_id.as_ref().as_ptr(),
        )
    };
    (program_id, data, len)
}
pub fn return_data_len() -> u64 {
    let len = unsafe { sol_get_return_data(0 as *const u8, 0, 0 as *const u8) };
    len
}
pub fn get_return_data() -> Option<(Pubkey, Vec<u8>)> {
    let program_id = Pubkey::default();
    let data_len_precomputed = return_data_len();
    if data_len_precomputed <= 0 {
        return None;
    }
    let data = vec![0u8; return_data_len() as usize];
    let data_len = unsafe {
        sol_get_return_data(
            data.as_ptr(),
            data.len() as u64,
            program_id.as_ref().as_ptr(),
        )
    };
    assert_eq!(data_len_precomputed, data_len);
    Some((program_id, data))
}
extern "C" {
    fn sol_keccak256(values_addr: *const u8, values_len: u64, result_addr: *mut u8);
}
macro_rules! hash_impl {
    ($data:ident, $hash_fn:ident) => {{
        let result = [0u8; 32];
        unsafe {
            $hash_fn(
                $data.as_ptr() as *const u8,
                $data.len() as u64,
                result.as_ptr() as *mut u8,
            )
        };
        result
    }};
}
pub fn sol_keccak256_native(data: &[&[u8]]) -> [u8; 32] {
    hash_impl!(data, sol_keccak256)
}
extern "C" {
    fn sol_sha256(values_addr: *const u8, values_len: u64, result_addr: *mut u8);
}
pub fn sol_sha256_native(data: &[&[u8]]) -> [u8; 32] {
    hash_impl!(data, sol_sha256)
}
extern "C" {
    fn sol_blake3(values_addr: *const u8, values_len: u64, result_addr: *mut u8);
}
pub fn sol_blake3_native(data: &[&[u8]]) -> [u8; 32] {
    hash_impl!(data, sol_blake3)
}
extern "C" {
    fn sol_secp256k1_recover(
        hash_addr: *const u8,
        recovery_id_val: u64,
        signature_addr: *const u8,
        result_addr: *mut u8,
    );
}
pub fn secp256k1_recover_native(
    hash: &[u8; 32],
    recovery_id_val: u64,
    signature: &[u8; 64],
) -> [u8; 64] {
    let mut result = [0u8; 64];
    unsafe {
        sol_secp256k1_recover(
            hash.as_ptr(),
            recovery_id_val,
            signature.as_ptr(),
            result.as_mut_ptr(),
        )
    };
    result
}
extern "C" {
    fn sol_big_mod_exp(params_addr: *const u8, return_value_addr: *mut u8);
}
pub fn big_mod_exp_native<const N: usize>(params: &[u8; size_of::<u64>() * 6]) -> [u8; N] {
    let mut result = [0u8; N];
    unsafe { sol_big_mod_exp(params.as_ptr(), result.as_mut_ptr()) };
    result
}
pub fn big_mod_exp_3<const N: usize>(base: &[u8], exponent: &[u8], modulus: &[u8; N]) -> [u8; N] {
    const PARAM_COMPONENT_SIZE: usize = size_of::<u64>();
    let mut params = [0u8; PARAM_COMPONENT_SIZE * 6];
    for (idx, param) in [base, exponent, modulus].iter().enumerate() {
        let param_ptr = param.as_ptr() as u64;
        let param_len = param.len() as u64;
        let idx_ptr_base = idx * PARAM_COMPONENT_SIZE * 2;
        let idx_len_base = idx_ptr_base + PARAM_COMPONENT_SIZE;
        params[idx_ptr_base..idx_ptr_base + PARAM_COMPONENT_SIZE]
            .copy_from_slice(&param_ptr.to_le_bytes());
        params[idx_len_base..idx_len_base + PARAM_COMPONENT_SIZE]
            .copy_from_slice(&param_len.to_le_bytes());
    }
    let mut result = [0u8; N];
    unsafe { sol_big_mod_exp(params.as_ptr(), result.as_mut_ptr()) };
    result
}
