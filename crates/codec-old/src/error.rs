extern crate alloc;

use alloc::{fmt, string::String};
use core::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum CodecError {
    Overflow,
    Encoding(EncodingError),
    Decoding(DecodingError),
}

impl Display for CodecError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            CodecError::Overflow => write!(f, "Overflow error"),
            CodecError::Encoding(err) => write!(f, "Encoding error: {}", err),
            CodecError::Decoding(err) => write!(f, "Decoding error: {}", err),
        }
    }
}

#[derive(Debug)]
pub enum EncodingError {
    BufferTooSmall {
        required: usize,
        available: usize,
        details: String,
    },
    InvalidInputData(String),
}

impl Display for EncodingError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            EncodingError::BufferTooSmall {
                required,
                available,
                details,
            } => {
                write!(f, "Not enough space in the buf: required {} bytes, but only {} bytes available. {}", required, available, details)
            }
            EncodingError::InvalidInputData(msg) => {
                write!(f, "Invalid data provided for encoding: {}", msg)
            }
        }
    }
}

#[derive(Debug)]
pub enum DecodingError {
    InvalidSelector {
        expected: [u8; 4],
        found: [u8; 4],
    },
    InvalidData(String),
    BufferTooSmall {
        expected: usize,
        found: usize,
        msg: String,
    },
    BufferOverflow {
        msg: String,
    },
    UnexpectedEof,
    Overflow,
    ParseError(String),
}

impl Display for DecodingError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            DecodingError::InvalidSelector { expected, found } => {
                write!(
                    f,
                    "Invalid selector: expected {:?}, found {:?}",
                    expected, found
                )
            }
            DecodingError::InvalidData(msg) => {
                write!(f, "Invalid data encountered during decoding: {}", msg)
            }
            DecodingError::BufferTooSmall {
                expected,
                found,
                msg,
            } => {
                write!(
                    f,
                    "Not enough data in the buf: expected at least {} bytes, found {}. {}",
                    expected, found, msg
                )
            }
            DecodingError::BufferOverflow { msg } => write!(f, "Buffer overflow: {}", msg),
            DecodingError::UnexpectedEof => write!(f, "Unexpected end of buf"),
            DecodingError::Overflow => write!(f, "Overflow error"),
            DecodingError::ParseError(msg) => write!(f, "Parsing error: {}", msg),
        }
    }
}
