use fluentbase_evm::types::{
    exit_code_from_instruction_result, instruction_result_from_exit_code, ExecutionResult,
    InterruptionOutcome,
};
use fluentbase_sdk::{Bytes, ExitCode, B256, FUEL_DENOM_RATE, U256};
use revm_interpreter::{Gas, InstructionResult};

#[test]
fn maps_exit_codes_to_instruction_results() {
    assert_eq!(
        instruction_result_from_exit_code(ExitCode::Ok, true),
        InstructionResult::Stop
    );
    assert_eq!(
        instruction_result_from_exit_code(ExitCode::Ok, false),
        InstructionResult::Return
    );

    let cases = [
        (ExitCode::Panic, InstructionResult::Revert),
        (ExitCode::InterruptionCalled, InstructionResult::Stop),
        (ExitCode::RootCallOnly, InstructionResult::RootCallOnly),
        (
            ExitCode::MalformedBuiltinParams,
            InstructionResult::MalformedBuiltinParams,
        ),
        (
            ExitCode::CallDepthOverflow,
            InstructionResult::CallDepthOverflow,
        ),
        (
            ExitCode::NonNegativeExitCode,
            InstructionResult::NonNegativeExitCode,
        ),
        (ExitCode::UnknownError, InstructionResult::UnknownError),
        (
            ExitCode::InputOutputOutOfBounds,
            InstructionResult::InputOutputOutOfBounds,
        ),
        (
            ExitCode::PrecompileError,
            InstructionResult::PrecompileError,
        ),
        (
            ExitCode::NotSupportedBytecode,
            InstructionResult::OpcodeNotFound,
        ),
        (
            ExitCode::StateChangeDuringStaticCall,
            InstructionResult::StateChangeDuringStaticCall,
        ),
        (
            ExitCode::CreateContractSizeLimit,
            InstructionResult::CreateContractSizeLimit,
        ),
        (
            ExitCode::CreateContractCollision,
            InstructionResult::CreateCollision,
        ),
        (
            ExitCode::CreateContractStartingWithEF,
            InstructionResult::CreateContractStartingWithEF,
        ),
        (ExitCode::OutOfMemory, InstructionResult::MemoryOutOfBounds),
        (ExitCode::InsufficientBalance, InstructionResult::OutOfFunds),
        (
            ExitCode::UnreachableCodeReached,
            InstructionResult::UnreachableCodeReached,
        ),
        (
            ExitCode::MemoryOutOfBounds,
            InstructionResult::MemoryOutOfBounds,
        ),
        (
            ExitCode::TableOutOfBounds,
            InstructionResult::TableOutOfBounds,
        ),
        (
            ExitCode::IndirectCallToNull,
            InstructionResult::IndirectCallToNull,
        ),
        (
            ExitCode::IntegerDivisionByZero,
            InstructionResult::IntegerDivisionByZero,
        ),
        (
            ExitCode::IntegerOverflow,
            InstructionResult::IntegerOverflow,
        ),
        (
            ExitCode::BadConversionToInteger,
            InstructionResult::BadConversionToInteger,
        ),
        (ExitCode::StackOverflow, InstructionResult::StackOverflow),
        (ExitCode::BadSignature, InstructionResult::BadSignature),
        (ExitCode::OutOfFuel, InstructionResult::OutOfFuel),
        (
            ExitCode::UnknownExternalFunction,
            InstructionResult::UnknownExternalFunction,
        ),
        (
            ExitCode::UnexpectedFatalExecutionFailure,
            InstructionResult::FatalExternalError,
        ),
        (
            ExitCode::MissingStorageSlot,
            InstructionResult::InvalidOperandOOG,
        ),
    ];

    for (exit_code, expected) in cases {
        assert_eq!(instruction_result_from_exit_code(exit_code, true), expected);
    }
}

#[test]
fn maps_instruction_results_to_exit_codes() {
    let cases = [
        (InstructionResult::Return, ExitCode::Ok),
        (InstructionResult::Stop, ExitCode::Ok),
        (InstructionResult::SelfDestruct, ExitCode::Ok),
        (InstructionResult::Revert, ExitCode::Panic),
        (InstructionResult::CallTooDeep, ExitCode::CallDepthOverflow),
        (InstructionResult::OutOfFunds, ExitCode::OutOfFuel),
        (
            InstructionResult::CreateInitCodeStartingEF00,
            ExitCode::CreateContractStartingWithEF,
        ),
        (
            InstructionResult::InvalidEOFInitCode,
            ExitCode::CreateContractSizeLimit,
        ),
        (
            InstructionResult::InvalidExtDelegateCallTarget,
            ExitCode::StateChangeDuringStaticCall,
        ),
        (InstructionResult::OutOfGas, ExitCode::OutOfFuel),
        (InstructionResult::MemoryOOG, ExitCode::OutOfFuel),
        (InstructionResult::MemoryLimitOOG, ExitCode::OutOfFuel),
        (InstructionResult::PrecompileOOG, ExitCode::OutOfFuel),
        (InstructionResult::InvalidOperandOOG, ExitCode::OutOfFuel),
        (InstructionResult::ReentrancySentryOOG, ExitCode::OutOfFuel),
        (
            InstructionResult::OpcodeNotFound,
            ExitCode::NotSupportedBytecode,
        ),
        (
            InstructionResult::CallNotAllowedInsideStatic,
            ExitCode::StateChangeDuringStaticCall,
        ),
        (
            InstructionResult::StateChangeDuringStaticCall,
            ExitCode::StateChangeDuringStaticCall,
        ),
        (
            InstructionResult::InvalidFEOpcode,
            ExitCode::NotSupportedBytecode,
        ),
        (
            InstructionResult::InvalidJump,
            ExitCode::NotSupportedBytecode,
        ),
        (
            InstructionResult::NotActivated,
            ExitCode::NotSupportedBytecode,
        ),
        (InstructionResult::StackUnderflow, ExitCode::StackOverflow),
        (InstructionResult::StackOverflow, ExitCode::StackOverflow),
        (
            InstructionResult::OutOfOffset,
            ExitCode::InputOutputOutOfBounds,
        ),
        (
            InstructionResult::CreateCollision,
            ExitCode::CreateContractCollision,
        ),
        (
            InstructionResult::OverflowPayment,
            ExitCode::IntegerOverflow,
        ),
        (
            InstructionResult::PrecompileError,
            ExitCode::PrecompileError,
        ),
        (InstructionResult::NonceOverflow, ExitCode::UnknownError),
        (
            InstructionResult::CreateContractSizeLimit,
            ExitCode::CreateContractSizeLimit,
        ),
        (
            InstructionResult::CreateContractStartingWithEF,
            ExitCode::CreateContractStartingWithEF,
        ),
        (
            InstructionResult::CreateInitCodeSizeLimit,
            ExitCode::CreateContractSizeLimit,
        ),
        (
            InstructionResult::FatalExternalError,
            ExitCode::UnexpectedFatalExecutionFailure,
        ),
        (
            InstructionResult::InvalidImmediateEncoding,
            ExitCode::UnreachableCodeReached,
        ),
        (InstructionResult::RootCallOnly, ExitCode::RootCallOnly),
        (
            InstructionResult::MalformedBuiltinParams,
            ExitCode::MalformedBuiltinParams,
        ),
        (
            InstructionResult::CallDepthOverflow,
            ExitCode::CallDepthOverflow,
        ),
        (
            InstructionResult::NonNegativeExitCode,
            ExitCode::NonNegativeExitCode,
        ),
        (InstructionResult::UnknownError, ExitCode::UnknownError),
        (
            InstructionResult::InputOutputOutOfBounds,
            ExitCode::InputOutputOutOfBounds,
        ),
        (
            InstructionResult::UnreachableCodeReached,
            ExitCode::UnreachableCodeReached,
        ),
        (
            InstructionResult::MemoryOutOfBounds,
            ExitCode::MemoryOutOfBounds,
        ),
        (
            InstructionResult::TableOutOfBounds,
            ExitCode::TableOutOfBounds,
        ),
        (
            InstructionResult::IndirectCallToNull,
            ExitCode::IndirectCallToNull,
        ),
        (
            InstructionResult::IntegerDivisionByZero,
            ExitCode::IntegerDivisionByZero,
        ),
        (
            InstructionResult::IntegerOverflow,
            ExitCode::IntegerOverflow,
        ),
        (
            InstructionResult::BadConversionToInteger,
            ExitCode::BadConversionToInteger,
        ),
        (InstructionResult::BadSignature, ExitCode::BadSignature),
        (InstructionResult::OutOfFuel, ExitCode::OutOfFuel),
        (
            InstructionResult::UnknownExternalFunction,
            ExitCode::UnknownExternalFunction,
        ),
    ];

    for (result, expected) in cases {
        assert_eq!(exit_code_from_instruction_result(result), expected);
    }
}

#[test]
fn converts_interruption_outcomes() {
    let output = Bytes::from(vec![0xabu8; 32]);
    let outcome = InterruptionOutcome {
        output: output.clone(),
        gas: Gas::new(123),
        exit_code: ExitCode::Ok,
        halted_frame: false,
    };
    assert_eq!(outcome.instruction_result(), InstructionResult::Return);

    let interpreter_result = outcome.clone().into_interpreter_result();
    assert_eq!(interpreter_result.result, InstructionResult::Return);
    assert_eq!(interpreter_result.output, output);
    assert_eq!(interpreter_result.gas, Gas::new(123));

    assert_eq!(outcome.clone().into_b256(), B256::from([0xabu8; 32]));
    assert_eq!(outcome.into_u256(), U256::from_le_bytes::<32>([0xabu8; 32]));

    let empty = InterruptionOutcome::default();
    assert_eq!(empty.instruction_result(), InstructionResult::Stop);
}

#[test]
fn computes_chargeable_fuel_with_saturation() {
    let result = ExecutionResult {
        committed_gas: Gas::new(100),
        gas: Gas::new(40),
        ..Default::default()
    };
    assert_eq!(result.chargeable_fuel(), 60 * FUEL_DENOM_RATE);

    let no_charge = ExecutionResult {
        committed_gas: Gas::new(40),
        gas: Gas::new(100),
        ..Default::default()
    };
    assert_eq!(no_charge.chargeable_fuel(), 0);

    let overflow = ExecutionResult {
        committed_gas: Gas::new(u64::MAX),
        gas: Gas::new(0),
        ..Default::default()
    };
    assert_eq!(overflow.chargeable_fuel(), u64::MAX);
}
