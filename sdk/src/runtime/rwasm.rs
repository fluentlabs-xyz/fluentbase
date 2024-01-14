use crate::{LowLevelRwasmSDK, LowLevelSDK};
use fluentbase_runtime::instruction::{rwasm_compile::RwasmCompile, rwasm_transact::RwasmTransact};

impl LowLevelRwasmSDK for LowLevelSDK {
    fn rwasm_compile(input: &[u8], output: &mut [u8]) -> i32 {
        match RwasmCompile::fn_impl(input, output.len() as u32) {
            Ok(result) => {
                output[0..result.len()].copy_from_slice(&result);
                0
            }
            Err(err_code) => err_code,
        }
    }

    fn rwasm_transact(
        bytecode: &[u8],
        input: &[u8],
        output: &mut [u8],
        state: u32,
        fuel: u32,
    ) -> i32 {
        match RwasmTransact::fn_impl(bytecode, input, state, fuel, output.len() as u32) {
            Ok(result) => {
                output[0..result.len()].copy_from_slice(&result);
                0
            }
            Err(err_code) => err_code,
        }
    }
}
