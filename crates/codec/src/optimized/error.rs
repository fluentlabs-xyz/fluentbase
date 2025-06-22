use core::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum CodecError {
    /// Not enough data in buffer
    BufferTooSmall { expected: usize, actual: usize },
    /// Overflow in calculations
    Overflow,
    /// Invalid format or data
    InvalidData(&'static str),
    /// Dynamic type in packed mode
    DynamicTypeInPackedMode,
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BufferTooSmall { expected, actual } => {
                write!(f, "buffer too small: expected {}, got {}", expected, actual)
            }
            Self::Overflow => write!(f, "numeric overflow"),
            Self::InvalidData(msg) => write!(f, "invalid data: {}", msg),
            Self::DynamicTypeInPackedMode => {
                write!(f, "dynamic types not supported in packed mode")
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for CodecError {}
