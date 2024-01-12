use crate::{Runtime, RuntimeContext};
use fluentbase_rwasm::{
    common::Trap,
    rwasm::{Compiler, CompilerConfig},
    Caller,
};

pub struct RwasmCompile;

impl RwasmCompile {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        input_offset: u32,
        input_len: u32,
        output_offset: u32,
        output_len: u32,
    ) -> Result<i32, Trap> {
        let input = caller.read_memory(input_offset, input_len);
        match Self::fn_impl(input, output_len) {
            Ok(output) => {
                caller.write_memory(output_offset, &output);
                Ok(0)
            }
            Err(err) => Ok(err),
        }
    }

    pub fn fn_impl(input: &[u8], output_len: u32) -> Result<Vec<u8>, i32> {
        let import_linker = Runtime::<()>::new_linker();
        let mut compiler = Compiler::new_with_linker(
            input.as_ref(),
            CompilerConfig::default().fuel_consume(true),
            Some(&import_linker),
        )
        .map_err(|err| err.into_i32())?;
        let output = compiler.finalize().map_err(|err| err.into_i32())?;
        if output_len < output.len() as u32 {
            return Err(output.len() as i32);
        }
        Ok(output)
    }
}
