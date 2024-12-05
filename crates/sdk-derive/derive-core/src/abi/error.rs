#[derive(Debug, thiserror::Error)]
pub enum ABIError {
    #[error("Invalid type conversion: {0}")]
    TypeConversion(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Deserialization error: {0}")]
    Deserialization(String),

    #[error("Unsupported type: {0}")]
    UnsupportedType(String),

    #[error("Unsupported pattern: {0}")]
    UnsupportedPattern(String),

    #[error("Syntax error: {0}")]
    SyntaxError(String),

    #[error("Internal error: {0}")]
    InternalError(String),
}

pub type ABIResult<T> = std::result::Result<T, ABIError>;
