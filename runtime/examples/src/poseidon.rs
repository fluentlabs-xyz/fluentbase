use fluentbase_sdk::{crypto_poseidon, sys_read};

pub fn main() {
    let mut input = [0u8; 11]; // "hello world"
    sys_read(input.as_mut_ptr(), 0, input.len() as u32);
    const EXPECTED_LEN: usize = 32;
    let mut output = [0u8; EXPECTED_LEN];
    let len = crypto_poseidon(&input, &mut output);
    if len as usize != EXPECTED_LEN {
        panic!("output len!={EXPECTED_LEN:?}");
    }
}
