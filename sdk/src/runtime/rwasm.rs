use crate::{RwasmPlatformSDK, SDK};
use alloc::vec;
use fluentbase_runtime::{ExitCode, Runtime, RuntimeContext, SysFuncIdx};
use fluentbase_rwasm::{
    instruction_set,
    rwasm::{Compiler, CompilerConfig, FuncOrExport},
};

impl RwasmPlatformSDK for SDK {
    fn rwasm_compile(input: &[u8], output: &mut [u8]) -> i32 {
        let import_linker = Runtime::<()>::new_linker();
        let compiler = Compiler::new_with_linker(
            input.as_ref(),
            CompilerConfig::default()
                .fuel_consume(true)
                .translate_sections(true),
            Some(&import_linker),
        );
        if compiler.is_err() {
            return -100;
        }
        let mut compiler = compiler.unwrap();
        let res = compiler.translate(FuncOrExport::StateRouter(
            vec![FuncOrExport::Export("main"), FuncOrExport::Export("deploy")],
            instruction_set! {
                Call(SysFuncIdx::SYS_STATE)
            },
        ));
        if res.is_err() {
            return -101;
        }
        let res = compiler.finalize();
        if res.is_err() {
            return -102;
        }
        let rwasm_bytecode = res.unwrap();
        if rwasm_bytecode.len() <= output.len() {
            let len = rwasm_bytecode.len();
            output[0..len].copy_from_slice(rwasm_bytecode.as_slice());
        }
        rwasm_bytecode.len() as i32
    }

    fn rwasm_transact(
        bytecode: &[u8],
        input: &[u8],
        output: &mut [u8],
        state: u32,
        fuel_limit: u32,
    ) -> i32 {
        let import_linker = Runtime::<()>::new_linker();
        let ctx = RuntimeContext::<()>::new(bytecode)
            .with_input(input.to_vec())
            .with_state(state)
            .with_fuel_limit(fuel_limit);
        let result = Runtime::<()>::run_with_context(ctx, &import_linker);
        if result.is_err() {
            return ExitCode::TransactError.into();
        }
        let execution_result = result.unwrap();
        let execution_output = execution_result.data().output();
        if execution_output.len() > output.len() {
            return ExitCode::TransactOutputOverflow.into();
        }
        let len = execution_output.len();
        output[0..len].copy_from_slice(execution_output.as_slice());
        execution_result.data().exit_code()
    }
}

#[cfg(test)]
mod test {
    use crate::{RwasmPlatformSDK, SDK};
    use alloc::vec;
    use fluentbase_runtime::{STATE_DEPLOY, STATE_MAIN};
    use hex_literal::hex;

    #[test]
    fn test_greeting() {
        let wasm_binary = include_bytes!("../../../examples/bin/greeting.wasm");
        let mut output = vec![0u8; 1024 * 1024];
        SDK::rwasm_compile(wasm_binary, output.as_mut_slice());
    }

    #[test]
    fn test_keccak256() {
        let wasm_binary = include_bytes!("../../../examples/bin/keccak256.wasm");
        let mut output = vec![0u8; 1024 * 1024];
        let code_len = SDK::rwasm_compile(wasm_binary, output.as_mut_slice());
        let mut result: [u8; 32] = [0; 32];
        SDK::with_test_state(STATE_MAIN);
        let exit_code = SDK::rwasm_transact(
            &output.as_slice()[0..code_len as usize],
            "Hello, World".as_bytes(),
            &mut result,
            STATE_MAIN,
            100_000,
        );
        assert_eq!(exit_code, 0);
        assert_eq!(
            result,
            hex!("a04a451028d0f9284ce82243755e245238ab1e4ecf7b9dd8bf4734d9ecfd0529")
        )
    }
}
