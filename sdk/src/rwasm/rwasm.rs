use crate::{RwasmPlatformSDK, SDK};

extern "C" {
    fn _rwasm_compile(
        input_ptr: *const u8,
        input_len: i32,
        output_ptr: *mut u8,
        output_len: i32,
    ) -> i32;
    fn _rwasm_transact(
        code_offset: *const u8,
        code_len: i32,
        input_offset: *const u8,
        input_len: i32,
        output_offset: *mut u8,
        output_len: i32,
        state: i32,
        fuel_limit: i32,
    ) -> i32;
}

impl RwasmPlatformSDK for SDK {
    #[inline(always)]
    fn rwasm_compile(input: &[u8], output: &mut [u8]) -> i32 {
        unsafe {
            _rwasm_compile(
                input.as_ptr(),
                input.len() as i32,
                output.as_mut_ptr(),
                output.len() as i32,
            )
        }
    }

    #[inline(always)]
    fn rwasm_transact(
        bytecode: &[u8],
        input: &[u8],
        output: &mut [u8],
        state: u32,
        fuel_limit: u32,
    ) -> i32 {
        unsafe {
            _rwasm_transact(
                bytecode.as_ptr(),
                bytecode.len() as i32,
                input.as_ptr(),
                input.len() as i32,
                output.as_mut_ptr(),
                output.len() as i32,
                state as i32,
                fuel_limit as i32,
            )
        }
    }
}
