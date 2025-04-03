use crate::evm::gas::Gas;
use fluentbase_sdk::Bytes;

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub enum InstructionResult {
    // Success Codes
    #[default]
    /// Execution should continue to the next one.
    Continue = 0x00,
    /// Encountered a `STOP` opcode
    Stop,
    /// Return from the current call.
    Return,
    /// Self-destruct the current contract.
    SelfDestruct,
    /// Return a contract (used in contract creation).
    ReturnContract,

    // Revert Codes
    /// Revert the transaction.
    Revert = 0x10,
    /// Exceeded maximum call depth.
    CallTooDeep,
    /// Insufficient funds for transfer.
    OutOfFunds,
    /// Revert if `CREATE`/`CREATE2` starts with `0xEF00`.
    CreateInitCodeStartingEF00,
    /// Invalid EVM Object Format (EOF) init code.
    InvalidEOFInitCode,
    /// `ExtDelegateCall` calling a non EOF contract.
    InvalidExtDelegateCallTarget,

    // Action Codes
    /// Indicates a call or contract creation.
    CallOrCreate = 0x20,

    // Error Codes
    /// Out of gas error.
    OutOfGas = 0x50,
    /// Out of gas error encountered during memory expansion.
    MemoryOOG,
    /// The memory limit of the EVM has been exceeded.
    MemoryLimitOOG,
    /// Out of gas error encountered during the execution of a precompiled contract.
    PrecompileOOG,
    /// Out of gas error encountered while calling an invalid operand.
    InvalidOperandOOG,
    /// Unknown or invalid opcode.
    OpcodeNotFound,
    /// Invalid `CALL` with value transfer in static context.
    CallNotAllowedInsideStatic,
    /// Invalid state modification in static call.
    StateChangeDuringStaticCall,
    /// An undefined bytecode value encountered during execution.
    InvalidFEOpcode,
    /// Invalid jump destination. Dynamic jumps points to invalid not jumpdest opcode.
    InvalidJump,
    /// The feature or opcode is not activated in this version of the EVM.
    NotActivated,
    /// Attempting to pop a value from an empty stack.
    StackUnderflow,
    /// Attempting to push a value onto a full stack.
    StackOverflow,
    /// Invalid memory or storage offset.
    OutOfOffset,
    /// Address collision during contract creation.
    CreateCollision,
    /// Payment amount overflow.
    OverflowPayment,
    /// Error in precompiled contract execution.
    PrecompileError,
    /// Nonce overflow.
    NonceOverflow,
    /// Exceeded contract size limit during creation.
    CreateContractSizeLimit,
    /// Created contract starts with invalid bytes (`0xEF`).
    CreateContractStartingWithEF,
    /// Exceeded init code size limit (EIP-3860:  Limit and meter initcode).
    CreateInitCodeSizeLimit,
    /// Fatal external error. Returned by database.
    FatalExternalError,
    /// `RETURNCONTRACT` called outside init EOF code.
    ReturnContractInNotInitEOF,
    /// Legacy contract is calling opcode that is enabled only in EOF.
    EOFOpcodeDisabledInLegacy,
    /// Stack overflow in EOF subroutine function calls.
    EOFFunctionStackOverflow,
    /// Aux data overflow, new aux data is larger than `u16` max size.
    EofAuxDataOverflow,
    /// Aux data is smaller then already present data size.
    EofAuxDataTooSmall,
    /// `EXT*CALL` target address needs to be padded with 0s.
    InvalidEXTCALLTarget,
}

#[macro_export]
macro_rules! return_ok {
    () => {
        $crate::evm::result::InstructionResult::Continue
            | $crate::evm::result::InstructionResult::Stop
            | $crate::evm::result::InstructionResult::Return
            | $crate::evm::result::InstructionResult::SelfDestruct
            | $crate::evm::result::InstructionResult::ReturnContract
    };
}

#[macro_export]
macro_rules! return_revert {
    () => {
        $crate::evm::result::InstructionResult::Revert
            | $crate::evm::result::InstructionResult::CallTooDeep
            | $crate::evm::result::InstructionResult::OutOfFunds
            | $crate::evm::result::InstructionResult::InvalidEOFInitCode
            | $crate::evm::result::InstructionResult::CreateInitCodeStartingEF00
            | $crate::evm::result::InstructionResult::InvalidExtDelegateCallTarget
    };
}

#[macro_export]
macro_rules! return_error {
    () => {
        $crate::evm::result::InstructionResult::OutOfGas
            | $crate::evm::result::InstructionResult::MemoryOOG
            | $crate::evm::result::InstructionResult::MemoryLimitOOG
            | $crate::evm::result::InstructionResult::PrecompileOOG
            | $crate::evm::result::InstructionResult::InvalidOperandOOG
            | $crate::evm::result::InstructionResult::OpcodeNotFound
            | $crate::evm::result::InstructionResult::CallNotAllowedInsideStatic
            | $crate::evm::result::InstructionResult::StateChangeDuringStaticCall
            | $crate::evm::result::InstructionResult::InvalidFEOpcode
            | $crate::evm::result::InstructionResult::InvalidJump
            | $crate::evm::result::InstructionResult::NotActivated
            | $crate::evm::result::InstructionResult::StackUnderflow
            | $crate::evm::result::InstructionResult::StackOverflow
            | $crate::evm::result::InstructionResult::OutOfOffset
            | $crate::evm::result::InstructionResult::CreateCollision
            | $crate::evm::result::InstructionResult::OverflowPayment
            | $crate::evm::result::InstructionResult::PrecompileError
            | $crate::evm::result::InstructionResult::NonceOverflow
            | $crate::evm::result::InstructionResult::CreateContractSizeLimit
            | $crate::evm::result::InstructionResult::CreateContractStartingWithEF
            | $crate::evm::result::InstructionResult::CreateInitCodeSizeLimit
            | $crate::evm::result::InstructionResult::FatalExternalError
            | $crate::evm::result::InstructionResult::ReturnContractInNotInitEOF
            | $crate::evm::result::InstructionResult::EOFOpcodeDisabledInLegacy
            | $crate::evm::result::InstructionResult::EOFFunctionStackOverflow
            | $crate::evm::result::InstructionResult::EofAuxDataTooSmall
            | $crate::evm::result::InstructionResult::EofAuxDataOverflow
            | $crate::evm::result::InstructionResult::InvalidEXTCALLTarget
    };
}

impl InstructionResult {
    /// Returns whether the result is a success.
    #[inline]
    pub const fn is_ok(self) -> bool {
        matches!(self, crate::return_ok!())
    }

    /// Returns whether the result is a revert.
    #[inline]
    pub const fn is_revert(self) -> bool {
        matches!(self, crate::return_revert!())
    }

    /// Returns whether the result is an error.
    #[inline]
    pub const fn is_error(self) -> bool {
        matches!(self, return_error!())
    }
}

/// The result of an interpreter operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterpreterResult {
    /// The result of the instruction execution.
    pub result: InstructionResult,
    /// The output of the instruction execution.
    pub output: Bytes,
    /// The gas usage information.
    pub gas: Gas,
}

impl InterpreterResult {
    /// Returns whether the instruction result is a success.
    #[inline]
    pub const fn is_ok(&self) -> bool {
        self.result.is_ok()
    }

    /// Returns whether the instruction result is a revert.
    #[inline]
    pub const fn is_revert(&self) -> bool {
        self.result.is_revert()
    }

    /// Returns whether the instruction result is an error.
    #[inline]
    pub const fn is_error(&self) -> bool {
        self.result.is_error()
    }
}

#[cfg(test)]
mod tests {
    use super::InstructionResult;

    #[test]
    fn all_results_are_covered() {
        match InstructionResult::Continue {
            return_error!() => {}
            return_revert!() => {}
            return_ok!() => {}
            InstructionResult::CallOrCreate => {}
        }
    }

    #[test]
    fn test_results() {
        let ok_results = vec![
            InstructionResult::Continue,
            InstructionResult::Stop,
            InstructionResult::Return,
            InstructionResult::SelfDestruct,
        ];

        for result in ok_results {
            assert!(result.is_ok());
            assert!(!result.is_revert());
            assert!(!result.is_error());
        }

        let revert_results = vec![
            InstructionResult::Revert,
            InstructionResult::CallTooDeep,
            InstructionResult::OutOfFunds,
        ];

        for result in revert_results {
            assert!(!result.is_ok());
            assert!(result.is_revert());
            assert!(!result.is_error());
        }

        let error_results = vec![
            InstructionResult::OutOfGas,
            InstructionResult::MemoryOOG,
            InstructionResult::MemoryLimitOOG,
            InstructionResult::PrecompileOOG,
            InstructionResult::InvalidOperandOOG,
            InstructionResult::OpcodeNotFound,
            InstructionResult::CallNotAllowedInsideStatic,
            InstructionResult::StateChangeDuringStaticCall,
            InstructionResult::InvalidFEOpcode,
            InstructionResult::InvalidJump,
            InstructionResult::NotActivated,
            InstructionResult::StackUnderflow,
            InstructionResult::StackOverflow,
            InstructionResult::OutOfOffset,
            InstructionResult::CreateCollision,
            InstructionResult::OverflowPayment,
            InstructionResult::PrecompileError,
            InstructionResult::NonceOverflow,
            InstructionResult::CreateContractSizeLimit,
            InstructionResult::CreateContractStartingWithEF,
            InstructionResult::CreateInitCodeSizeLimit,
            InstructionResult::FatalExternalError,
        ];

        for result in error_results {
            assert!(!result.is_ok());
            assert!(!result.is_revert());
            assert!(result.is_error());
        }
    }
}
