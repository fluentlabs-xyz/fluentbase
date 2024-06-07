use fluentbase_sdk::{LowLevelSDK, SharedAPI};

pub fn deploy() {}

pub fn main() {
    LowLevelSDK::write("Hello, World".as_bytes());
}
