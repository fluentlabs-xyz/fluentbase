use fluentbase_sdk::{crypto_secp256k1_verify, sys_read};

pub fn main() {
    const DIGEST_OFFSET: i32 = 0;
    const DIGEST_LEN: i32 = 32;
    const SIG_OFFSET: i32 = DIGEST_OFFSET + DIGEST_LEN;
    const SIG_LEN: i32 = 64;
    const RECID_OFFSET: i32 = SIG_OFFSET + SIG_LEN;
    const RECID_LEN: i32 = 1;
    const PK_EXPECTED_OFFSET: i32 = RECID_OFFSET + RECID_LEN;
    const PK_EXPECTED_LEN: i32 = 33;

    let mut input = [0u8; (DIGEST_LEN + SIG_LEN + RECID_LEN + PK_EXPECTED_LEN) as usize];
    sys_read(input.as_mut_ptr(), 0, input.len() as u32);
    const EXPECTED_RES: i32 = 1;

    let res = crypto_secp256k1_verify(
        input.as_ptr(),
        DIGEST_LEN as usize,
        input.as_ptr().wrapping_add(SIG_OFFSET as usize),
        SIG_LEN as usize,
        input.as_ptr().wrapping_add(PK_EXPECTED_OFFSET as usize),
        PK_EXPECTED_LEN as usize,
        input[RECID_OFFSET as usize] as i32,
    );
    if res != EXPECTED_RES {
        panic!("res!={EXPECTED_RES:?}");
    }
}
