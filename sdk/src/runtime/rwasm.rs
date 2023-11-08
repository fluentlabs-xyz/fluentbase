use crate::{RwasmPlatformSDK, SDK};
use fluentbase_runtime::{ExitCode, Runtime, RuntimeContext, SysFuncIdx};
use fluentbase_rwasm::engine::bytecode::Instruction;
use fluentbase_rwasm::rwasm::{Compiler, FuncOrExport};

impl RwasmPlatformSDK for SDK {
    fn rwasm_compile(input: &[u8], output: &mut [u8]) -> i32 {
        let import_linker = Runtime::new_linker();
        let mut compiler = Compiler::new_with_linker(input.as_ref(), Some(&import_linker)).unwrap();
        compiler
            .translate(Some(FuncOrExport::StateRouter(
                vec![FuncOrExport::Export("deploy"), FuncOrExport::Export("main")],
                Instruction::Call(SysFuncIdx::SYS_STATE.into()),
            )))
            .unwrap();
        let rwasm_bytecode = compiler.finalize().unwrap();
        if rwasm_bytecode.len() <= output.len() {
            output.copy_from_slice(rwasm_bytecode.as_slice());
        } else {
            output.copy_from_slice(&rwasm_bytecode.as_slice()[0..output.len()]);
        }
        rwasm_bytecode.len() as i32
    }

    fn rwasm_transact(bytecode: &[u8], input: &[u8], output: &mut [u8], state: u32) -> i32 {
        let import_linker = Runtime::new_linker();
        let result = Runtime::run_with_context(
            RuntimeContext::new(bytecode)
                .with_input(input.to_vec())
                .with_state(state),
            &import_linker,
        );
        if result.is_err() {
            return ExitCode::TransactError.into();
        }
        let execution_result = result.unwrap();
        let execution_output = execution_result.data().output();
        if execution_output.len() > output.len() {
            return ExitCode::TransactOutputOverflow.into();
        }
        output.copy_from_slice(execution_output.as_slice());
        execution_result.data().exit_code()
    }
}
