use crate::{
    rwasm::{
        bindings::{_rwasm_compile, _rwasm_transact},
        LowLevelSDK,
    },
    sdk::LowLevelRwasmSDK,
};

impl LowLevelRwasmSDK for LowLevelSDK {
    fn rwasm_compile(input: &[u8], output: &mut [u8]) -> i32 {
        unsafe {
            _rwasm_compile(
                input.as_ptr(),
                input.len() as u32,
                output.as_mut_ptr(),
                output.len() as u32,
            )
        }
    }

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
                bytecode.len() as u32,
                input.as_ptr(),
                input.len() as u32,
                output.as_mut_ptr(),
                output.len() as u32,
                state,
                fuel_limit,
            )
        }
    }
}
