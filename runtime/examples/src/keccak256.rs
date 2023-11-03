use fluentbase_sdk::{crypto_keccak256, sys_read};

pub fn main() {
    let mut input = [0u8; 11]; // "hello world"
    sys_read(input.as_mut_ptr(), 0, input.len() as u32);
    const EXPECTED_LEN: i32 = 32;
    const OUTPUT_OFFSET: i32 = 0;
    let len = crypto_keccak256(input.as_mut_ptr() as i32, input.len() as i32, OUTPUT_OFFSET);
    if len != EXPECTED_LEN {
        panic!("output len!={EXPECTED_LEN:?}");
    }
}
