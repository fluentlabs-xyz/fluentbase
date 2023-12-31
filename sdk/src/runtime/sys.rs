use crate::{SysPlatformSDK, SDK};
use alloc::{vec, vec::Vec};

lazy_static::lazy_static! {
    static ref INPUT: std::sync::Mutex<Vec<u8>> = std::sync::Mutex::new(vec![]);
    static ref OUTPUT: std::sync::Mutex<Vec<u8>> = std::sync::Mutex::new(vec![]);
    static ref STATE: std::sync::Mutex<u32> = std::sync::Mutex::new(0);
}

#[allow(dead_code)]
impl SDK {
    pub fn with_test_input(input: Vec<u8>) {
        INPUT.lock().unwrap().clear();
        INPUT.lock().unwrap().extend(&input);
    }

    pub fn get_test_output() -> Vec<u8> {
        let result = OUTPUT.lock().unwrap().clone();
        OUTPUT.lock().unwrap().clear();
        result
    }

    pub(crate) fn with_test_state(state: u32) {
        *STATE.lock().unwrap() = state;
    }
}

impl SysPlatformSDK for SDK {
    fn sys_read(target: &mut [u8], offset: u32) -> u32 {
        let input = &INPUT.lock().unwrap();
        let input = &input[(offset as usize)..(offset as usize + target.len())];
        target.copy_from_slice(&input);
        target.len() as u32
    }

    fn sys_write(value: &[u8]) {
        OUTPUT.lock().unwrap().extend(value);
    }

    fn sys_halt(exit_code: i32) {
        unreachable!("program has exited with code: {}", exit_code)
    }

    fn sys_state() -> u32 {
        *STATE.lock().unwrap()
    }
}
