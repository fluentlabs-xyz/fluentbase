use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};

extern "C" {
    fn __get_stack_pointer() -> u32;
    fn __set_stack_pointer(sp: u32);
}

pub fn deploy() {}

pub fn main() {
    unsafe {
        LowLevelSDK::exit(__get_stack_pointer() as i32);
    }
}
