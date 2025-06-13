use crate::{instruction::exec::SysExecResumable, ExecutionResult};
use fluentbase_types::{BytecodeOrHash, Bytes, B256};
use rwasm::TrapCode;

pub struct RuntimeContext {
    // context inputs
    pub(crate) bytecode: BytecodeOrHash,
    pub(crate) fuel_limit: u64,
    pub(crate) state: u32,
    pub(crate) call_depth: u32,
    pub(crate) input: Bytes,
    pub(crate) disable_fuel: bool,
    // TODO(dmitry123): "check function `remember_runtime`, it's not correct"
    pub(crate) call_counter: u32,
    // context outputs
    pub(crate) execution_result: ExecutionResult,
    pub(crate) resumable_context: Option<SysExecResumable>,
}

impl Default for RuntimeContext {
    fn default() -> Self {
        Self {
            bytecode: BytecodeOrHash::default(),
            fuel_limit: 0,
            state: 0,
            input: Bytes::default(),
            call_depth: 0,
            execution_result: ExecutionResult::default(),
            disable_fuel: false,
            call_counter: 0,
            resumable_context: None,
        }
    }
}

impl RuntimeContext {
    pub fn root(fuel_limit: u64) -> Self {
        Self::default()
            .with_fuel_limit(fuel_limit)
            .with_call_depth(0)
    }

    pub fn new<I: Into<BytecodeOrHash>>(bytecode: I) -> Self {
        Self {
            bytecode: bytecode.into(),
            ..Default::default()
        }
    }

    pub fn new_with_hash(bytecode_hash: B256) -> Self {
        Self {
            bytecode: BytecodeOrHash::Hash(bytecode_hash),
            ..Default::default()
        }
    }

    pub fn with_bytecode(mut self, bytecode: BytecodeOrHash) -> Self {
        self.bytecode = bytecode;
        self
    }

    pub fn with_input<I: Into<Bytes>>(mut self, input_data: I) -> Self {
        self.input = input_data.into();
        self
    }

    pub fn change_input(&mut self, input_data: Bytes) {
        self.input = input_data;
    }

    pub fn with_state(mut self, state: u32) -> Self {
        self.state = state;
        self
    }

    pub fn with_fuel_limit(mut self, fuel_limit: u64) -> Self {
        self.fuel_limit = fuel_limit;
        self
    }

    pub fn with_call_depth(mut self, depth: u32) -> Self {
        self.call_depth = depth;
        self
    }

    pub fn with_disable_fuel(mut self, disable_fuel: bool) -> Self {
        self.disable_fuel = disable_fuel;
        self
    }

    pub fn without_fuel(mut self) -> Self {
        self.disable_fuel = true;
        self
    }

    pub fn depth(&self) -> u32 {
        self.call_depth
    }

    pub fn exit_code(&self) -> i32 {
        self.execution_result.exit_code
    }

    pub fn input(&self) -> &Bytes {
        &self.input
    }

    pub fn input_size(&self) -> u32 {
        self.input.len() as u32
    }

    pub fn output(&self) -> &Vec<u8> {
        &self.execution_result.output
    }

    pub fn output_mut(&mut self) -> &mut Vec<u8> {
        &mut self.execution_result.output
    }

    pub fn fuel_limit(&self) -> u64 {
        self.fuel_limit
    }

    pub fn return_data(&self) -> &Vec<u8> {
        &self.execution_result.return_data
    }

    pub fn into_return_data(self) -> Bytes {
        self.execution_result.return_data.into()
    }

    pub fn return_data_mut(&mut self) -> &mut Vec<u8> {
        &mut self.execution_result.return_data
    }

    pub fn state(&self) -> u32 {
        self.state
    }

    pub fn clear_output(&mut self) {
        self.execution_result.output.clear();
    }

    pub fn try_consume_fuel(&mut self, fuel: u64) -> Result<(), TrapCode> {
        let consumed_fuel = self
            .execution_result
            .fuel_consumed
            .checked_add(fuel)
            .unwrap_or(u64::MAX);
        if !self.disable_fuel {
            if consumed_fuel > self.fuel_limit {
                return Err(TrapCode::OutOfFuel);
            }
        }
        self.execution_result.fuel_consumed = consumed_fuel;
        Ok(())
    }

    pub fn refund_fuel(&mut self, fuel: i64) {
        self.execution_result.fuel_refunded += fuel;
    }

    pub fn remaining_fuel(&self) -> u64 {
        self.fuel_limit - self.execution_result.fuel_consumed
    }
}
