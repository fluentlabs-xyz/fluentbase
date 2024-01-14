use crate::{
    rwasm::{
        bindings::{_sys_halt, _sys_input_size, _sys_read, _sys_state, _sys_write},
        LowLevelSDK,
    },
    sdk::LowLevelSysSDK,
};

impl LowLevelSysSDK for LowLevelSDK {
    fn sys_read(target: &mut [u8], offset: u32) {
        unsafe { _sys_read(target.as_mut_ptr(), offset, target.len() as u32) }
    }

    fn sys_input_size() -> u32 {
        unsafe { _sys_input_size() }
    }

    fn sys_write(value: &[u8]) {
        unsafe { _sys_write(value.as_ptr(), value.len() as u32) }
    }

    fn sys_halt(exit_code: i32) {
        unsafe { _sys_halt(exit_code) }
    }

    fn sys_state() -> u32 {
        unsafe { _sys_state() }
    }
}
