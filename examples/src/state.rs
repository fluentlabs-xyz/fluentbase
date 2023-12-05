use fluentbase_sdk::{SysPlatformSDK, SDK};

pub fn deploy() {
    let buf: [u8; 1] = [100];
    SDK::sys_write(&buf);
}

pub fn main() {
    let buf: [u8; 1] = [200];
    SDK::sys_write(&buf);
}
