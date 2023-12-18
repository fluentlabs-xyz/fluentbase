use fluentbase_sdk::{SysPlatformSDK, SDK};

extern "C" {
    fn __get_stack_pointer() -> u32;
}

pub fn deploy() {
    unimplemented!()
}

pub fn main() {
    unsafe {
        SDK::sys_halt(__get_stack_pointer() as i32);
    }
}
