use fluentbase_sdk::{SysPlatformSDK, SDK};

pub fn main() {
    let str = "Hello, World";
    SDK::sys_write_slice(str.as_bytes());
}
