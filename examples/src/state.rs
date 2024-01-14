use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};

pub fn deploy() {
    let buf: [u8; 1] = [100];
    LowLevelSDK::sys_write(&buf);
}

pub fn main() {
    let buf: [u8; 1] = [200];
    LowLevelSDK::sys_write(&buf);
}
