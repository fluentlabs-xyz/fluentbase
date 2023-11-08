use fluentbase_sdk::{SysPlatformSDK, SDK};

pub fn main() {
    let str = "Hello, World";
    SDK::sys_write(str.as_bytes());
}
