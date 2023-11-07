use crate::{RwasmPlatformSDK, SDK};

extern "C" {
    fn _rwasm_compile(
        input_ptr: *const u8,
        input_len: i32,
        output_ptr: *mut u8,
        output_len: i32,
    ) -> i32;
    fn _rwasm_transact(
        code_offset: i32,
        code_len: i32,
        input_offset: i32,
        input_len: i32,
        output_offset: i32,
        output_len: i32,
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
        code_offset: i32,
        code_len: i32,
        input_offset: i32,
        input_len: i32,
        output_offset: i32,
        output_len: i32,
    ) -> i32 {
        unsafe {
            _rwasm_transact(
                code_offset,
                code_len,
                input_offset,
                input_len,
                output_offset,
                output_len,
            )
        }
    }
}
