use crate::deploy_internal;
use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};

extern "C" {
    fn __get_stack_pointer() -> u32;
    fn __set_stack_pointer(sp: u32);
}

pub fn deploy() {
    deploy_internal(include_bytes!("../bin/stack.wasm"))
}

pub fn main() {
    unsafe {
        LowLevelSDK::sys_halt(__get_stack_pointer() as i32);
    }
}
