use crate::{instruction::exec::SysExecResumable, ExecutionResult};
use fluentbase_types::Bytes;
use std::mem::take;

#[derive(Debug, Clone)]
pub struct RuntimeContext {
    // context inputs
    pub(crate) disable_fuel: bool,
    pub(crate) state: u32,
    pub(crate) call_depth: u32,
    pub(crate) input: Bytes,
    // context outputs
    pub(crate) execution_result: ExecutionResult,
    pub(crate) resumable_context: Option<SysExecResumable>,
}

impl Default for RuntimeContext {
    fn default() -> Self {
        Self {
            state: 0,
            input: Bytes::default(),
            call_depth: 0,
            execution_result: ExecutionResult::default(),
            disable_fuel: false,
            resumable_context: None,
        }
    }
}

impl RuntimeContext {
    pub fn root() -> Self {
        Self::default().with_call_depth(0)
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

    pub fn return_data(&self) -> &Vec<u8> {
        &self.execution_result.return_data
    }

    pub fn into_return_data(self) -> Bytes {
        self.execution_result.return_data.into()
    }

    pub fn take_return_data(&mut self) -> Vec<u8> {
        take(&mut self.execution_result.return_data)
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

    // pub fn try_consume_fuel(&mut self, fuel: u64) -> Result<(), TrapCode> {
    //     let consumed_fuel = self
    //         .execution_result
    //         .fuel_consumed
    //         .checked_add(fuel)
    //         .unwrap_or(u64::MAX);
    //     if !self.disable_fuel {
    //         if consumed_fuel > self.fuel_limit {
    //             return Err(TrapCode::OutOfFuel);
    //         }
    //     }
    //     self.execution_result.fuel_consumed = consumed_fuel;
    //     Ok(())
    // }

    pub fn refund_fuel(&mut self, fuel: i64) {
        self.execution_result.fuel_refunded += fuel;
    }

    // pub fn remaining_fuel(&self) -> u64 {
    //     self.fuel_limit - self.execution_result.fuel_consumed
    // }
}
