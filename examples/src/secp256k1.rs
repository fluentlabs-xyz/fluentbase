use fluentbase_sdk::{LowLevelCryptoSDK, LowLevelSDK, LowLevelSysSDK};

pub fn main() {
    const DIGEST_OFFSET: usize = 0;
    const DIGEST_LEN: usize = 32;
    const SIG_OFFSET: usize = DIGEST_OFFSET + DIGEST_LEN;
    const SIG_LEN: usize = 64;
    const REC_ID_OFFSET: usize = SIG_OFFSET + SIG_LEN;
    const REC_ID_LEN: usize = 1;
    const PK_EXPECTED_OFFSET: usize = REC_ID_OFFSET + REC_ID_LEN;
    const PK_EXPECTED_LEN: usize = 65;

    let mut digest = [0u8; DIGEST_LEN];
    LowLevelSDK::sys_read(&mut digest, DIGEST_OFFSET as u32);
    let mut sig = [0u8; SIG_LEN];
    LowLevelSDK::sys_read(&mut sig, SIG_OFFSET as u32);
    let mut rec_id = [0u8; REC_ID_LEN];
    LowLevelSDK::sys_read(&mut rec_id, REC_ID_OFFSET as u32);
    let mut pk_expected = [0u8; PK_EXPECTED_LEN];
    LowLevelSDK::sys_read(&mut pk_expected, PK_EXPECTED_OFFSET as u32);
    let mut pk_output = [0u8; PK_EXPECTED_LEN];

    LowLevelSDK::crypto_ecrecover(&digest, &sig, &mut pk_output, rec_id[0]);
    if pk_expected != pk_output {
        panic!("verification failed")
    }
}
