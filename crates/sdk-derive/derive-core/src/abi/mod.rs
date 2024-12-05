mod error;
mod function;
mod parameter;

pub use error::{ABIError, ABIResult};
pub use function::FunctionABI;
use serde::Serialize;

// Core types
/// Represents Solidity contract ABI
#[derive(Debug, Clone, Serialize)]
pub enum ABI {
    /// Function definition in ABI
    Function(FunctionABI),
}
