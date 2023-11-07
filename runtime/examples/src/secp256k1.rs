use fluentbase_sdk::{EccPlatformSDK, SysPlatformSDK, SDK};

pub fn main() {
    const DIGEST_OFFSET: usize = 0;
    const DIGEST_LEN: usize = 32;
    const SIG_OFFSET: usize = DIGEST_OFFSET + DIGEST_LEN;
    const SIG_LEN: usize = 64;
    const REC_ID_OFFSET: usize = SIG_OFFSET + SIG_LEN;
    const REC_ID_LEN: usize = 1;
    const PK_EXPECTED_OFFSET: usize = REC_ID_OFFSET + REC_ID_LEN;
    const PK_EXPECTED_LEN: usize = 33;

    let mut digest = [0u8; DIGEST_LEN];
    SDK::sys_read_slice(&mut digest, DIGEST_OFFSET as u32);
    let mut sig = [0u8; SIG_LEN];
    SDK::sys_read_slice(&mut sig, SIG_OFFSET as u32);
    let mut rec_id = [0u8; REC_ID_LEN];
    SDK::sys_read_slice(&mut rec_id, REC_ID_OFFSET as u32);
    let mut pk_expected = [0u8; PK_EXPECTED_LEN];
    SDK::sys_read_slice(&mut pk_expected, PK_EXPECTED_OFFSET as u32);

    let res = SDK::ecc_secp256k1_verify(&digest, &sig, &pk_expected, rec_id[0]);
    if !res {
        unreachable!("verification failed")
    }
}
