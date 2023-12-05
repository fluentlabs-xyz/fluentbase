use crate::{SysPlatformSDK, SDK};
use alloc::{vec, vec::Vec};

lazy_static::lazy_static! {
    static ref INPUT: std::sync::Mutex<Vec<u8>> = std::sync::Mutex::new(vec![]);
}

impl SDK {
    pub fn with_test_input(input: Vec<u8>) {
        INPUT.lock().unwrap().extend(&input);
    }
}

impl SysPlatformSDK for SDK {
    fn sys_read(target: &mut [u8], offset: u32) -> u32 {
        let input = &INPUT.lock().unwrap();
        let input = &input[(offset as usize)..(offset as usize + target.len())];
        target.copy_from_slice(&input);
        target.len() as u32
    }

    fn sys_write(_value: &[u8]) {
        unreachable!("I think this function is not possible for runtime")
    }

    fn sys_halt(exit_code: i32) {
        unreachable!("program has exited with code: {}", exit_code)
    }

    fn sys_state() -> u32 {
        unreachable!("state is not known")
    }
}
