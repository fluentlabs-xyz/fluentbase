/// Precompile errors
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrecompileError {
    InvalidPublicKey,
    InvalidRecoveryId,
    InvalidSignature,
    InvalidDataOffsets,
    InvalidInstructionDataSize,
}

impl num_traits::FromPrimitive for PrecompileError {
    #[inline]
    fn from_i64(n: i64) -> Option<Self> {
        if n == PrecompileError::InvalidPublicKey as i64 {
            Some(PrecompileError::InvalidPublicKey)
        } else if n == PrecompileError::InvalidRecoveryId as i64 {
            Some(PrecompileError::InvalidRecoveryId)
        } else if n == PrecompileError::InvalidSignature as i64 {
            Some(PrecompileError::InvalidSignature)
        } else if n == PrecompileError::InvalidDataOffsets as i64 {
            Some(PrecompileError::InvalidDataOffsets)
        } else if n == PrecompileError::InvalidInstructionDataSize as i64 {
            Some(PrecompileError::InvalidInstructionDataSize)
        } else {
            None
        }
    }
    #[inline]
    fn from_u64(n: u64) -> Option<Self> {
        Self::from_i64(n as i64)
    }
}
impl<T> DecodeError<T> for PrecompileError {
    fn type_of() -> &'static str {
        "PrecompileError"
    }
}

impl num_traits::ToPrimitive for PrecompileError {
    #[inline]
    fn to_i64(&self) -> Option<i64> {
        Some(match *self {
            PrecompileError::InvalidPublicKey => PrecompileError::InvalidPublicKey as i64,
            PrecompileError::InvalidRecoveryId => PrecompileError::InvalidRecoveryId as i64,
            PrecompileError::InvalidSignature => PrecompileError::InvalidSignature as i64,
            PrecompileError::InvalidDataOffsets => PrecompileError::InvalidDataOffsets as i64,
            PrecompileError::InvalidInstructionDataSize => {
                PrecompileError::InvalidInstructionDataSize as i64
            }
        })
    }
    #[inline]
    fn to_u64(&self) -> Option<u64> {
        self.to_i64().map(|x| x as u64)
    }
}
