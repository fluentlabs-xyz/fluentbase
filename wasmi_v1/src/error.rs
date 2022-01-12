use super::errors::{
    GlobalError,
    InstantiationError,
    LinkerError,
    MemoryError,
    TableError,
    TranslationError,
};
use crate::Trap;
use core::{fmt, fmt::Display};

/// An error that may occur upon operating on Wasm modules or module instances.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// A global variable error.
    Global(GlobalError),
    /// A linear memory error.
    Memory(MemoryError),
    /// A table error.
    Table(TableError),
    /// A linker error.
    Linker(LinkerError),
    /// A Wasm to `wasmi` bytecode translation error.
    Translation(TranslationError),
    /// A module instantiation error.
    Instantiation(InstantiationError),
    /// A trap as defined by the WebAssembly specification.
    Trap(Trap),
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Trap(error) => Display::fmt(error, f),
            Error::Global(error) => Display::fmt(error, f),
            Error::Memory(error) => Display::fmt(error, f),
            Error::Table(error) => Display::fmt(error, f),
            Error::Linker(error) => Display::fmt(error, f),
            Error::Translation(error) => Display::fmt(error, f),
            Error::Instantiation(error) => Display::fmt(error, f),
        }
    }
}

impl From<Trap> for Error {
    fn from(error: Trap) -> Self {
        Self::Trap(error)
    }
}

impl From<GlobalError> for Error {
    fn from(error: GlobalError) -> Self {
        Self::Global(error)
    }
}

impl From<MemoryError> for Error {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

impl From<TableError> for Error {
    fn from(error: TableError) -> Self {
        Self::Table(error)
    }
}

impl From<LinkerError> for Error {
    fn from(error: LinkerError) -> Self {
        Self::Linker(error)
    }
}

impl From<TranslationError> for Error {
    fn from(error: TranslationError) -> Self {
        Self::Translation(error)
    }
}

impl From<InstantiationError> for Error {
    fn from(error: InstantiationError) -> Self {
        Self::Instantiation(error)
    }
}
