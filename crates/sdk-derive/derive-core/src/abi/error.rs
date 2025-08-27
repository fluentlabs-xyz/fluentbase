#[derive(Debug, thiserror::Error)]
pub enum ABIError {
    #[error("Invalid type: {0}")]
    InvalidType(Box<str>),

    #[error("Invalid type conversion: {0}")]
    TypeConversion(Box<str>),

    #[error("Serialization error: {0}")]
    Serialization(Box<str>),

    #[error("Deserialization error: {0}")]
    Deserialization(Box<str>),

    #[error("Unsupported type: {0}")]
    UnsupportedType(Box<str>),

    #[error("Unsupported pattern: {0}")]
    UnsupportedPattern(Box<str>),

    #[error("Syntax error: {0}")]
    Syntax(Box<str>),

    #[error("Internal error: {0}")]
    Internal(Box<str>),

    #[error("Artifacts error: {0}")]
    Artifacts(Box<str>),

    #[error("Config error: {0}")]
    Config(Box<str>),
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
