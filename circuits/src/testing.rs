use fluentbase_runtime::{ExecutionResult, Runtime, RuntimeError};

#[derive(Debug, Default)]
pub struct TestingContext {
    rwasm_bytecode: Vec<u8>,
    execution_result: Option<ExecutionResult>,
    input_data: Vec<u8>,
}

impl TestingContext {
    pub fn new(rwasm_bytecode: Vec<u8>) -> Self {
        Self {
            rwasm_bytecode,
            ..Default::default()
        }
    }

    pub fn with_input_data(&mut self, input_data: Vec<u8>) -> &mut Self {
        self.input_data = input_data;
        self
    }

    pub fn execution_result(&self) -> &ExecutionResult {
        self.execution_result.as_ref().unwrap()
    }

    pub fn run(&mut self) -> Result<(), RuntimeError> {
        let execution_result = Runtime::run(
            self.rwasm_bytecode.as_slice(),
            &self.input_data.clone(),
            10_000_000,
        )?;
        self.execution_result = Some(execution_result);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::testing::TestingContext;
    use fluentbase_rwasm::{instruction_set, rwasm::InstructionSet};

    fn test_ok(bytecode: InstructionSet) {
        let bytecode: Vec<u8> = bytecode.into();
        let mut testing_context = TestingContext::new(bytecode);
        testing_context.run().unwrap();
        let exec_res = testing_context.execution_result();
        exec_res.tracer();
    }

    #[test]
    fn test_add_three_numbers() {
        test_ok(instruction_set!(
            .op_i32_const(100)
            .op_i32_const(20)
            .op_i32_add()
            .op_i32_const(3)
            .op_i32_add()
            .op_drop()
            .op_return()
        ));
    }
}
