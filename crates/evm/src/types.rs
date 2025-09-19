//! Shared types for the interruptible interpreter.
//!
//! InterruptionOutcome carries host call results back into the VM;
//! InterruptionExtension stores per-interpreter state; ExecutionResult
//! summarizes final outcome with gas/fuel accounting.
use fluentbase_sdk::{Bytes, ExitCode, B256, FUEL_DENOM_RATE, U256};
use revm_interpreter::{interpreter::EthInterpreter, Gas, InstructionResult, InterpreterResult};

#[derive(Debug)]
/// Result of a host interruption (output, gas delta, and exit code).
pub struct InterruptionOutcome {
    pub output: Bytes,
    pub gas: Gas,
    pub exit_code: ExitCode,
}

impl InterruptionOutcome {
    pub fn instruction_result(&self) -> InstructionResult {
        match self.exit_code {
            ExitCode::Ok => InstructionResult::Return,
            ExitCode::Panic => InstructionResult::Revert,
            // There is no diff what error code to use, but it should be error code
            ExitCode::Err => InstructionResult::UnreachableCodeReached,
            ExitCode::PrecompileError => InstructionResult::PrecompileError,
            ExitCode::OutOfFuel => InstructionResult::OutOfGas,
            ec => unreachable!("unexpected exit code: {} ({})", ec.into_i32(), ec),
        }
    }

    pub fn into_interpreter_result(self) -> InterpreterResult {
        InterpreterResult {
            result: self.instruction_result(),
            output: self.output,
            gas: self.gas,
        }
    }

    pub fn into_b256(self) -> B256 {
        debug_assert_eq!(self.output.len(), 32);
        B256::from_slice(self.output.as_ref())
    }

    pub fn into_u256(self) -> U256 {
        debug_assert_eq!(self.output.len(), 32);
        U256::from_le_slice(self.output.as_ref())
    }
}

#[derive(Default)]
/// Extra per-interpreter state used during interruptions.
pub struct InterruptionExtension {
    pub interruption_outcome: Option<InterruptionOutcome>,
    pub committed_gas: Gas,
}

pub type InterruptingInterpreter = EthInterpreter<InterruptionExtension>;

/// Final result of EthVM::run_the_loop with gas/fuel details.
pub struct ExecutionResult {
    /// The result of the instruction execution.
    pub result: InstructionResult,
    /// The output of the instruction execution.
    pub output: Bytes,
    /// The gas already committed to the runtime (aka charged).
    pub committed_gas: Gas,
    /// The gas usage information.
    pub gas: Gas,
}

impl ExecutionResult {
    /// Fuel/refund delta to settle at the host based on committed vs. final gas.
    pub fn chargeable_fuel_and_refund(&self) -> (u64, i64) {
        let remaining_diff = self.committed_gas.remaining() - self.gas.remaining();
        let refunded_diff = self.gas.refunded() - self.committed_gas.refunded();
        (
            // TODO(dmitry123): Is it safe to mul here? What about debug mode?
            remaining_diff * FUEL_DENOM_RATE,
            refunded_diff * FUEL_DENOM_RATE as i64,
        )
    }
}
