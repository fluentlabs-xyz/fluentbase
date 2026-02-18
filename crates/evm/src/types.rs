//! Shared types for the interruptible interpreter.
//!
//! InterruptionOutcome carries host call results back into the VM;
//! InterruptionExtension stores per-interpreter state; ExecutionResult
//! summarizes an outcome with gas/fuel accounting.
use fluentbase_sdk::{Bytes, ExitCode, B256, FUEL_DENOM_RATE, U256};
use revm_interpreter::{interpreter::EthInterpreter, Gas, InstructionResult, InterpreterResult};

#[derive(Default, Debug, Clone)]
/// Result of a host interruption (output, gas delta, and exit code).
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct InterruptionOutcome {
    pub output: Bytes,
    pub gas: Gas,
    pub exit_code: ExitCode,
    pub halted_frame: bool,
}

pub fn instruction_result_from_exit_code(
    exit_code: ExitCode,
    is_empty_return_data: bool,
) -> InstructionResult {
    match exit_code {
        /* Basic Error Codes */
        ExitCode::Ok => {
            if is_empty_return_data {
                InstructionResult::Stop
            } else {
                InstructionResult::Return
            }
        }
        ExitCode::Panic => InstructionResult::Revert,
        ExitCode::InterruptionCalled => InstructionResult::Stop,
        /* Fluentbase Runtime Error Codes */
        ExitCode::RootCallOnly => InstructionResult::RootCallOnly,
        ExitCode::MalformedBuiltinParams => InstructionResult::MalformedBuiltinParams,
        ExitCode::CallDepthOverflow => InstructionResult::CallDepthOverflow,
        ExitCode::NonNegativeExitCode => InstructionResult::NonNegativeExitCode,
        ExitCode::UnknownError => InstructionResult::UnknownError,
        ExitCode::InputOutputOutOfBounds => InstructionResult::InputOutputOutOfBounds,
        ExitCode::PrecompileError => InstructionResult::PrecompileError,
        ExitCode::NotSupportedBytecode => InstructionResult::MalformedBuiltinParams,
        ExitCode::StateChangeDuringStaticCall => InstructionResult::StateChangeDuringStaticCall,
        ExitCode::CreateContractSizeLimit => InstructionResult::CreateContractSizeLimit,
        ExitCode::CreateContractCollision => InstructionResult::CreateCollision,
        ExitCode::CreateContractStartingWithEF => InstructionResult::CreateContractStartingWithEF,
        ExitCode::OutOfMemory => InstructionResult::MemoryOutOfBounds,
        /* Trap Error Codes */
        ExitCode::UnreachableCodeReached => InstructionResult::UnreachableCodeReached,
        ExitCode::MemoryOutOfBounds => InstructionResult::MemoryOutOfBounds,
        ExitCode::TableOutOfBounds => InstructionResult::TableOutOfBounds,
        ExitCode::IndirectCallToNull => InstructionResult::IndirectCallToNull,
        ExitCode::IntegerDivisionByZero => InstructionResult::IntegerDivisionByZero,
        ExitCode::IntegerOverflow => InstructionResult::IntegerOverflow,
        ExitCode::BadConversionToInteger => InstructionResult::BadConversionToInteger,
        ExitCode::StackOverflow => InstructionResult::StackOverflow,
        ExitCode::BadSignature => InstructionResult::BadSignature,
        ExitCode::OutOfFuel => InstructionResult::OutOfFuel,
        ExitCode::UnknownExternalFunction => InstructionResult::UnknownExternalFunction,
        /* System Fatal Error Codes */
        ExitCode::UnexpectedFatalExecutionFailure => InstructionResult::FatalExternalError,
        ExitCode::MissingStorageSlot => InstructionResult::InvalidOperandOOG,
    }
}

pub fn exit_code_from_instruction_result(result: InstructionResult) -> ExitCode {
    match result {
        InstructionResult::Return | InstructionResult::Stop | InstructionResult::SelfDestruct => {
            ExitCode::Ok
        }
        InstructionResult::Revert => ExitCode::Panic,
        InstructionResult::CallTooDeep => ExitCode::CallDepthOverflow,
        InstructionResult::OutOfFunds => ExitCode::OutOfFuel,
        InstructionResult::CreateInitCodeStartingEF00 => ExitCode::CreateContractStartingWithEF,
        InstructionResult::InvalidEOFInitCode => ExitCode::CreateContractSizeLimit,
        InstructionResult::InvalidExtDelegateCallTarget => ExitCode::StateChangeDuringStaticCall,
        InstructionResult::OutOfGas
        | InstructionResult::MemoryOOG
        | InstructionResult::MemoryLimitOOG
        | InstructionResult::PrecompileOOG
        | InstructionResult::InvalidOperandOOG
        | InstructionResult::ReentrancySentryOOG => ExitCode::OutOfFuel,
        InstructionResult::OpcodeNotFound => ExitCode::NotSupportedBytecode,
        InstructionResult::CallNotAllowedInsideStatic
        | InstructionResult::StateChangeDuringStaticCall => ExitCode::StateChangeDuringStaticCall,
        InstructionResult::InvalidFEOpcode => ExitCode::NotSupportedBytecode,
        InstructionResult::InvalidJump | InstructionResult::NotActivated => {
            ExitCode::NotSupportedBytecode
        }
        InstructionResult::StackUnderflow | InstructionResult::StackOverflow => {
            ExitCode::StackOverflow
        }
        InstructionResult::OutOfOffset => ExitCode::InputOutputOutOfBounds,
        InstructionResult::CreateCollision => ExitCode::CreateContractCollision,
        InstructionResult::OverflowPayment => ExitCode::IntegerOverflow,
        InstructionResult::PrecompileError => ExitCode::PrecompileError,
        InstructionResult::NonceOverflow => ExitCode::UnknownError,
        InstructionResult::CreateContractSizeLimit => ExitCode::CreateContractSizeLimit,
        InstructionResult::CreateContractStartingWithEF => ExitCode::CreateContractStartingWithEF,
        InstructionResult::CreateInitCodeSizeLimit => ExitCode::CreateContractSizeLimit,
        InstructionResult::FatalExternalError => ExitCode::UnexpectedFatalExecutionFailure,
        InstructionResult::RootCallOnly => ExitCode::RootCallOnly,
        InstructionResult::MalformedBuiltinParams => ExitCode::MalformedBuiltinParams,
        InstructionResult::CallDepthOverflow => ExitCode::CallDepthOverflow,
        InstructionResult::NonNegativeExitCode => ExitCode::NonNegativeExitCode,
        InstructionResult::UnknownError => ExitCode::UnknownError,
        InstructionResult::InputOutputOutOfBounds => ExitCode::InputOutputOutOfBounds,
        InstructionResult::UnreachableCodeReached => ExitCode::UnreachableCodeReached,
        InstructionResult::MemoryOutOfBounds => ExitCode::MemoryOutOfBounds,
        InstructionResult::TableOutOfBounds => ExitCode::TableOutOfBounds,
        InstructionResult::IndirectCallToNull => ExitCode::IndirectCallToNull,
        InstructionResult::IntegerDivisionByZero => ExitCode::IntegerDivisionByZero,
        InstructionResult::IntegerOverflow => ExitCode::IntegerOverflow,
        InstructionResult::BadConversionToInteger => ExitCode::BadConversionToInteger,
        InstructionResult::BadSignature => ExitCode::BadSignature,
        InstructionResult::OutOfFuel => ExitCode::OutOfFuel,
        InstructionResult::UnknownExternalFunction => ExitCode::UnknownExternalFunction,
    }
}

impl InterruptionOutcome {
    pub fn instruction_result(&self) -> InstructionResult {
        instruction_result_from_exit_code(self.exit_code, self.output.is_empty())
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

/// Extra per-interpreter state used during interruptions.
#[derive(Default, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct InterruptionExtension {
    pub interruption_outcome: Option<InterruptionOutcome>,
    pub committed_gas: Gas,
}

pub type InterruptingInterpreter = EthInterpreter<InterruptionExtension>;

/// Final result of `EthVM::run_the_loop` with gas/fuel details.
#[derive(Debug, Default)]
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
    pub fn chargeable_fuel(&self) -> u64 {
        let remaining_diff = self.committed_gas.remaining() - self.gas.remaining();
        // TODO(dmitry123): Is it safe to mul here? What about debug mode?
        remaining_diff * FUEL_DENOM_RATE
    }
}
