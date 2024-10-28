use thiserror::Error;

#[derive(Debug, Error)]
pub enum RouterError {
    #[error("Invalid router mode: {0}")]
    InvalidMode(String),
    #[error("Invalid function signature: {0}")]
    InvalidSignature(String),
    #[error("Method parsing error: {0}")]
    MethodParseError(String),
    #[error(transparent)]
    SynError(#[from] syn::Error),
}

impl From<RouterError> for syn::Error {
    fn from(err: RouterError) -> Self {
        match err {
            RouterError::InvalidMode(msg)
            | RouterError::InvalidSignature(msg)
            | RouterError::MethodParseError(msg) => {
                syn::Error::new(proc_macro2::Span::call_site(), msg)
            }
            RouterError::SynError(e) => e,
        }
    }
}
