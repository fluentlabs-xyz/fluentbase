use proc_macro2::Span;
use thiserror::Error;

#[derive(Debug)]
pub struct ErrorDetails {
    pub span: Span,
    pub note: Option<String>,
    pub help: Option<String>,
}

#[derive(Debug, Error)]
pub enum RouterError {
    #[error("Invalid router mode: {message}")]
    InvalidMode {
        message: String,
        details: ErrorDetails,
    },

    #[error("Invalid function signature: {message}")]
    InvalidSignature {
        message: String,
        details: ErrorDetails,
    },

    #[error("Method parsing error: {message}")]
    MethodParseError {
        message: String,
        details: ErrorDetails,
    },

    #[error(transparent)]
    SynError(#[from] syn::Error),
}

impl RouterError {
    pub fn span(&self) -> Span {
        match self {
            RouterError::InvalidMode { details, .. }
            | RouterError::InvalidSignature { details, .. }
            | RouterError::MethodParseError { details, .. } => details.span,
            RouterError::SynError(e) => e.span(),
        }
    }

    pub fn note(&self) -> Option<&String> {
        match self {
            RouterError::InvalidMode { details, .. }
            | RouterError::InvalidSignature { details, .. }
            | RouterError::MethodParseError { details, .. } => details.note.as_ref(),
            _ => None,
        }
    }

    pub fn help(&self) -> Option<&String> {
        match self {
            RouterError::InvalidMode { details, .. }
            | RouterError::InvalidSignature { details, .. }
            | RouterError::MethodParseError { details, .. } => details.help.as_ref(),
            _ => None,
        }
    }

    pub fn invalid_mode(message: &str, span: Span, note: Option<&str>, help: Option<&str>) -> Self {
        RouterError::InvalidMode {
            message: message.to_string(),
            details: ErrorDetails {
                span,
                note: note.map(|s| s.to_string()),
                help: help.map(|s| s.to_string()),
            },
        }
    }

    pub fn invalid_signature(
        message: &str,
        span: Span,
        note: Option<&str>,
        help: Option<&str>,
    ) -> Self {
        RouterError::InvalidSignature {
            message: message.to_string(),
            details: ErrorDetails {
                span,
                note: note.map(|s| s.to_string()),
                help: help.map(|s| s.to_string()),
            },
        }
    }

    pub fn method_parse_error(
        message: &str,
        span: Span,
        note: Option<&str>,
        help: Option<&str>,
    ) -> Self {
        RouterError::MethodParseError {
            message: message.to_string(),
            details: ErrorDetails {
                span,
                note: note.map(|s| s.to_string()),
                help: help.map(|s| s.to_string()),
            },
        }
    }

    pub fn to_syn_error(&self) -> syn::Error {
        let mut base_error = syn::Error::new(self.span(), self.to_string());

        if let RouterError::InvalidMode { details, .. }
        | RouterError::InvalidSignature { details, .. }
        | RouterError::MethodParseError { details, .. } = self
        {
            details.add_details_to_error(&mut base_error);
        }

        base_error
    }
}

impl ErrorDetails {
    pub fn add_details_to_error(&self, error: &mut syn::Error) {
        if let Some(note) = &self.note {
            error.combine(syn::Error::new(self.span, format!("Note: {}", note)));
        }
        if let Some(help) = &self.help {
            error.combine(syn::Error::new(self.span, format!("Help: {}", help)));
        }
    }
}

impl From<RouterError> for syn::Error {
    fn from(err: RouterError) -> Self {
        err.to_syn_error()
    }
}
