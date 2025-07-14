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
