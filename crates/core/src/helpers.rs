use alloc::{boxed::Box, string::ToString, vec, vec::Vec};
use fluentbase_sdk::{create_import_linker, ExitCode, SysFuncIdx::STATE, STATE_DEPLOY, STATE_MAIN};
use revm_interpreter::InstructionResult;
use rwasm::{
    engine::{bytecode::Instruction, RwasmConfig, StateRouterConfig},
    rwasm::{BinaryFormat, BinaryFormatWriter, RwasmModule},
};
use solana_ee_core::svm::error::{EbpfError, ProgramResult};

#[inline(always)]
pub fn wasm2rwasm(wasm_binary: &[u8]) -> Result<Vec<u8>, ExitCode> {
    let mut config = RwasmModule::default_config(None);
    config.rwasm_config(RwasmConfig {
        state_router: Some(StateRouterConfig {
            states: Box::new([
                ("deploy".to_string(), STATE_DEPLOY),
                ("main".to_string(), STATE_MAIN),
            ]),
            opcode: Instruction::Call((STATE as u32).into()),
        }),
        entrypoint_name: None,
        import_linker: Some(create_import_linker()),
        wrap_import_functions: true,
    });
    let rwasm_module = RwasmModule::compile_with_config(wasm_binary, &config)
        .map_err(|_| ExitCode::CompilationError)?;
    let length = rwasm_module.encoded_length();
    let mut rwasm_bytecode = vec![0u8; length];
    let mut binary_format_writer = BinaryFormatWriter::new(&mut rwasm_bytecode);
    rwasm_module
        .write_binary(&mut binary_format_writer)
        .expect("failed to encode rwasm bytecode");
    Ok(rwasm_bytecode)
}

#[macro_export]
macro_rules! result_value {
    ($result:expr) => {
        match $result {
            Ok(v) => v,
            Err(v) => v,
        }
    };
}

#[cfg(feature = "debug-print")]
#[macro_export]
macro_rules! debug_log {
    ($msg:tt) => {{
        #[cfg(target_arch = "wasm32")]
        unsafe { fluentbase_sdk::rwasm::_debug_log($msg.as_ptr(), $msg.len() as u32) }
        #[cfg(feature = "std")]
        println!("{}", $msg);
    }};
    ($($arg:tt)*) => {{
        let msg = alloc::format!($($arg)*);
        debug_log!(msg);
    }};
}
#[cfg(not(feature = "debug-print"))]
#[macro_export]
macro_rules! debug_log {
    ($msg:tt) => {{}};
    ($($arg:tt)*) => {{}};
}

pub fn evm_error_from_exit_code(exit_code: ExitCode) -> InstructionResult {
    match exit_code {
        ExitCode::Ok => InstructionResult::Stop,
        ExitCode::Panic => InstructionResult::Revert,
        ExitCode::CallDepthOverflow => InstructionResult::CallTooDeep,
        ExitCode::InsufficientBalance => InstructionResult::OutOfFunds,
        ExitCode::OutOfGas => InstructionResult::OutOfGas,
        ExitCode::OpcodeNotFound => InstructionResult::OpcodeNotFound,
        ExitCode::WriteProtection => InstructionResult::StateChangeDuringStaticCall,
        ExitCode::InvalidEfOpcode => InstructionResult::InvalidFEOpcode,
        ExitCode::InvalidJump => InstructionResult::InvalidJump,
        // ExitCode::NotActivated => InstructionResult::NotActivated,
        ExitCode::StackUnderflow => InstructionResult::StackUnderflow,
        ExitCode::StackOverflow => InstructionResult::StackOverflow,
        ExitCode::OutputOverflow => InstructionResult::OutOfOffset,
        ExitCode::CreateCollision => InstructionResult::CreateCollision,
        ExitCode::OverflowPayment => InstructionResult::OverflowPayment,
        ExitCode::PrecompileError => InstructionResult::PrecompileError,
        ExitCode::NonceOverflow => InstructionResult::NonceOverflow,
        ExitCode::ContractSizeLimit => InstructionResult::CreateContractSizeLimit,
        ExitCode::CreateContractStartingWithEF => InstructionResult::CreateContractStartingWithEF,
        ExitCode::FatalExternalError => InstructionResult::FatalExternalError,
        // TODO(dmitry123): "what's proper unknown error code mapping?"
        _ => InstructionResult::OutOfGas,
    }
}

pub fn exit_code_from_evm_error(evm_error: InstructionResult) -> ExitCode {
    match evm_error {
        InstructionResult::Continue
        | InstructionResult::Stop
        | InstructionResult::Return
        | InstructionResult::SelfDestruct
        | InstructionResult::CallOrCreate => ExitCode::Ok,
        InstructionResult::Revert => ExitCode::Panic,
        InstructionResult::CallTooDeep => ExitCode::CallDepthOverflow,
        InstructionResult::OutOfFunds => ExitCode::InsufficientBalance,
        InstructionResult::OutOfGas
        | InstructionResult::MemoryOOG
        | InstructionResult::MemoryLimitOOG
        | InstructionResult::PrecompileOOG
        | InstructionResult::InvalidOperandOOG => ExitCode::OutOfGas,
        InstructionResult::OpcodeNotFound => ExitCode::OpcodeNotFound,
        InstructionResult::CallNotAllowedInsideStatic
        | InstructionResult::StateChangeDuringStaticCall => ExitCode::WriteProtection,
        InstructionResult::InvalidFEOpcode => ExitCode::InvalidEfOpcode,
        InstructionResult::InvalidJump => ExitCode::InvalidJump,
        InstructionResult::StackUnderflow => ExitCode::StackUnderflow,
        InstructionResult::StackOverflow => ExitCode::StackOverflow,
        InstructionResult::OutOfOffset => ExitCode::OutputOverflow,
        InstructionResult::CreateCollision => ExitCode::CreateCollision,
        InstructionResult::OverflowPayment => ExitCode::OverflowPayment,
        InstructionResult::PrecompileError => ExitCode::PrecompileError,
        InstructionResult::NonceOverflow => ExitCode::NonceOverflow,
        InstructionResult::CreateContractSizeLimit | InstructionResult::CreateInitCodeSizeLimit => {
            ExitCode::ContractSizeLimit
        }
        InstructionResult::CreateContractStartingWithEF => ExitCode::CreateContractStartingWithEF,
        InstructionResult::FatalExternalError => ExitCode::FatalExternalError,
        _ => ExitCode::UnknownError,
    }
}

pub fn svm_result_from_exit_code(exit_code: ExitCode) -> ProgramResult {
    match exit_code {
        ExitCode::Ok => ProgramResult::Ok(0),
        ExitCode::Panic => ProgramResult::Err(EbpfError::InvalidInstruction),
        ExitCode::CallDepthOverflow => ProgramResult::Err(EbpfError::InvalidInstruction),
        ExitCode::InsufficientBalance => ProgramResult::Err(EbpfError::InvalidInstruction),
        ExitCode::OutOfGas => ProgramResult::Err(EbpfError::InvalidInstruction),
        ExitCode::OpcodeNotFound => ProgramResult::Err(EbpfError::InvalidInstruction),
        ExitCode::WriteProtection => ProgramResult::Err(EbpfError::InvalidInstruction),
        ExitCode::InvalidEfOpcode => ProgramResult::Err(EbpfError::InvalidInstruction),
        ExitCode::InvalidJump => ProgramResult::Err(EbpfError::InvalidInstruction),
        // ExitCode::NotActivated => ProgramResult::Err(EbpfError::InvalidInstruction),
        ExitCode::StackUnderflow => ProgramResult::Err(EbpfError::InvalidInstruction),
        ExitCode::StackOverflow => ProgramResult::Err(EbpfError::InvalidInstruction),
        ExitCode::OutputOverflow => ProgramResult::Err(EbpfError::InvalidInstruction),
        ExitCode::CreateCollision => ProgramResult::Err(EbpfError::InvalidInstruction),
        ExitCode::OverflowPayment => ProgramResult::Err(EbpfError::InvalidInstruction),
        ExitCode::PrecompileError => ProgramResult::Err(EbpfError::InvalidInstruction),
        ExitCode::NonceOverflow => ProgramResult::Err(EbpfError::InvalidInstruction),
        ExitCode::ContractSizeLimit => ProgramResult::Err(EbpfError::InvalidInstruction),
        ExitCode::CreateContractStartingWithEF => ProgramResult::Err(EbpfError::InvalidInstruction),
        ExitCode::FatalExternalError => ProgramResult::Err(EbpfError::InvalidInstruction),
        // TODO: proper unknown error code mapping
        _ => ProgramResult::Err(EbpfError::UnsupportedInstruction),
    }
}

pub fn exit_code_from_svm_result(result: ProgramResult) -> ExitCode {
    match result {
        ProgramResult::Ok(_) => ExitCode::Ok,
        ProgramResult::Err(error) => match error {
            EbpfError::ElfError(_) => ExitCode::UnknownError,
            EbpfError::FunctionAlreadyRegistered(_) => ExitCode::UnresolvedFunction,
            EbpfError::CallDepthExceeded => ExitCode::UnknownError,
            EbpfError::ExitRootCallFrame => ExitCode::UnknownError,
            EbpfError::DivideByZero => ExitCode::UnknownError,
            EbpfError::DivideOverflow => ExitCode::UnknownError,
            EbpfError::ExecutionOverrun => ExitCode::UnknownError,
            EbpfError::CallOutsideTextSegment => ExitCode::UnknownError,
            EbpfError::ExceededMaxInstructions => ExitCode::UnknownError,
            EbpfError::JitNotCompiled => ExitCode::UnknownError,
            EbpfError::InvalidVirtualAddress(_) => ExitCode::UnknownError,
            EbpfError::InvalidMemoryRegion(_) => ExitCode::UnknownError,
            EbpfError::AccessViolation(_, _, _, _) => ExitCode::UnknownError,
            EbpfError::StackAccessViolation(_, _, _, _) => ExitCode::UnknownError,
            EbpfError::InvalidInstruction => ExitCode::UnknownError,
            EbpfError::UnsupportedInstruction => ExitCode::UnknownError,
            EbpfError::ExhaustedTextSegment(_) => ExitCode::UnknownError,
            EbpfError::LibcInvocationFailed(_, _, _) => ExitCode::UnknownError,
            EbpfError::VerifierError(_) => ExitCode::UnknownError,
            EbpfError::SyscallError(_) => ExitCode::UnknownError,
            _ => ExitCode::UnknownError,
        },
    }
}
