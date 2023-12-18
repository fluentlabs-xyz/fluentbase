use fluentbase_sdk::{SysPlatformSDK, SDK};

extern "C" {
    fn __get_stack_pointer() -> u32;
    fn __set_stack_pointer(sp: u32);
}

pub fn deploy() {}

pub fn main() {
    unsafe {
        SDK::sys_halt(__get_stack_pointer() as i32);
    }
}
