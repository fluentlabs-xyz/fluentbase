use fluentbase_sdk::ExitCode;
use solana_program::entrypoint::ProgramResult;
use solana_rbpf::error::EbpfError;
// use solana_ee_core::svm::error::{EbpfError, ProgramResult};

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
