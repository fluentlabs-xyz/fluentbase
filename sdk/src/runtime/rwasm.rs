use crate::{RwasmPlatformSDK, SDK};

impl RwasmPlatformSDK for SDK {
    fn rwasm_compile(input: &[u8], output: &mut [u8]) -> i32 {
        todo!("not implemented yet")
    }

    fn rwasm_transact(
        code_offset: i32,
        code_len: i32,
        input_offset: i32,
        input_len: i32,
        output_offset: i32,
        output_len: i32,
    ) -> i32 {
        todo!("not implemented yet")
    }
}
