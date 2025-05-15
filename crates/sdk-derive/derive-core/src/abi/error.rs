#[derive(Debug, thiserror::Error)]
pub enum ABIError {
    #[error("Invalid type: {0}")]
    InvalidType(String),

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
    Syntax(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Artifacts error: {0}")]
    Artifacts(String),

    #[error("Config error: {0}")]
    Config(String),
}

impl From<ABIError> for syn::Error {
    fn from(error: ABIError) -> Self {
        syn::Error::new(proc_macro2::Span::call_site(), error.to_string())
    }
}

impl From<ABIError> for proc_macro_error::Diagnostic {
    fn from(error: ABIError) -> Self {
        proc_macro_error::Diagnostic::new(proc_macro_error::Level::Error, error.to_string())
    }
}
