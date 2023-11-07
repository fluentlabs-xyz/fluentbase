use fluentbase_sdk::{crypto_keccak256, sys_read};

pub fn main() {
    let mut input = [0u8; 11]; // "hello world"
    sys_read(input.as_mut_ptr(), 0, input.len() as u32);
    let mut output = [0u8; 32];
    crypto_keccak256(&input, &mut output);
}
