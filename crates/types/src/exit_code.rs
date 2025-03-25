use rwasm::core::{Trap, TrapCode};
use strum_macros::{Display, FromRepr};

#[derive(Default, Debug, Copy, Clone, Eq, PartialEq, Display, FromRepr)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(i32)]
pub enum ExitCode {
    // warning: when adding new codes doesn't forget to add them to impls below
    #[default]
    Ok = 0,
    Panic = -1,
    Err = -2,
    // fluentbase error codes
    RootCallOnly = -1002,
    MalformedBuiltinParams = -1003,
    CallDepthOverflow = -1004,
    NonNegativeExitCode = -1005,
    UnknownError = -1006,
    InputOutputOutOfBounds = -1007,
    // trap error codes
    UnreachableCodeReached = -2001,
    MemoryOutOfBounds = -2002,
    TableOutOfBounds = -2003,
    IndirectCallToNull = -2004,
    IntegerDivisionByZero = -2005,
    IntegerOverflow = -2006,
    BadConversionToInteger = -2007,
    StackOverflow = -2008,
    BadSignature = -2009,
    OutOfFuel = -2010,
    GrowthOperationLimited = -2011,
    UnresolvedFunction = -2013,
    // Continue = 0x00,
    // Stop,
    // Return,
    // SelfDestruct,
    // ReturnContract,
    //
    // // Revert Codes
    // Revert = 0x10,
    // CallTooDeep,
    // OutOfFunds,
    // CreateInitCodeStartingEF00,
    // InvalidEOFInitCode,
    // InvalidExtDelegateCallTarget,
    //
    // // Error Codes
    // OutOfGas = 0x50,
    // MemoryOOG,
    // MemoryLimitOOG,
    // PrecompileOOG,
    // InvalidOperandOOG,
    // OpcodeNotFound,
    // CallNotAllowedInsideStatic,
    // StateChangeDuringStaticCall,
    // InvalidFEOpcode,
    // InvalidJump,
    // NotActivated,
    // StackUnderflow,
    // OutOfOffset,
    // CreateCollision,
    // OverflowPayment,
    // PrecompileError,
    // NonceOverflow,
    // CreateContractSizeLimit,
    // CreateContractStartingWithEF,
    // CreateInitCodeSizeLimit,
    // FatalExternalError,
    // ReturnContractInNotInitEOF,
    // EOFOpcodeDisabledInLegacy,
    // EOFFunctionStackOverflow,
    // EofAuxDataOverflow,
    // EofAuxDataTooSmall,
    // InvalidEXTCALLTarget,
}

pub trait UnwrapExitCode<T> {
    fn unwrap_exit_code(self) -> T;
}

impl<T> UnwrapExitCode<T> for Result<T, ExitCode> {
    fn unwrap_exit_code(self) -> T {
        match self {
            Ok(res) => res,
            Err(err) => panic!("exit code: {} ({})", err, err.into_i32()),
        }
    }
}

impl From<i32> for ExitCode {
    fn from(value: i32) -> Self {
        Self::from_repr(value).unwrap_or(ExitCode::UnknownError)
    }
}

impl Into<i32> for ExitCode {
    fn into(self) -> i32 {
        self as i32
    }
}

impl ExitCode {
    pub const fn is_ok(&self) -> bool {
        self.into_i32() == Self::Ok.into_i32()
    }

    pub const fn is_error(&self) -> bool {
        self.into_i32() != Self::Ok.into_i32()
    }

    /// Returns whether the result is a revert.
    pub const fn is_revert(self) -> bool {
        self.into_i32() != Self::Ok.into_i32()
    }

    pub const fn into_i32(self) -> i32 {
        self as i32
    }

    pub fn into_trap(self) -> Trap {
        Trap::i32_exit(self as i32)
    }
}

impl From<TrapCode> for ExitCode {
    fn from(value: TrapCode) -> Self {
        Self::from(&value)
    }
}

impl From<&TrapCode> for ExitCode {
    fn from(value: &TrapCode) -> Self {
        match value {
            TrapCode::UnreachableCodeReached => ExitCode::UnreachableCodeReached,
            TrapCode::MemoryOutOfBounds => ExitCode::MemoryOutOfBounds,
            TrapCode::TableOutOfBounds => ExitCode::TableOutOfBounds,
            TrapCode::IndirectCallToNull => ExitCode::IndirectCallToNull,
            TrapCode::IntegerDivisionByZero => ExitCode::IntegerDivisionByZero,
            TrapCode::IntegerOverflow => ExitCode::IntegerOverflow,
            TrapCode::BadConversionToInteger => ExitCode::BadConversionToInteger,
            TrapCode::StackOverflow => ExitCode::StackOverflow,
            TrapCode::BadSignature => ExitCode::BadSignature,
            TrapCode::OutOfFuel => ExitCode::OutOfFuel,
            TrapCode::GrowthOperationLimited => ExitCode::GrowthOperationLimited,
            TrapCode::UnresolvedFunction => ExitCode::UnresolvedFunction,
        }
    }
}

impl Into<Trap> for ExitCode {
    fn into(self) -> Trap {
        self.into_trap()
    }
}

impl From<Trap> for ExitCode {
    fn from(value: Trap) -> Self {
        ExitCode::from(&value)
    }
}

impl From<&Trap> for ExitCode {
    fn from(value: &Trap) -> Self {
        if let Some(trap_code) = value.trap_code() {
            return ExitCode::from(trap_code);
        }
        if let Some(exit_code) = value.i32_exit_status() {
            return ExitCode::from(exit_code);
        }
        ExitCode::UnknownError
    }
}
