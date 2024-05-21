use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};

pub fn deploy() {}

pub fn main() {
    LowLevelSDK::sys_write("Hello, World".as_bytes());
}
