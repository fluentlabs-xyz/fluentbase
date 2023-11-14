//! EVM opcode implementations.

#[macro_use]
mod macros;

pub mod arithmetic;
pub mod bitwise;
pub mod control;
pub mod host_env;
pub mod i256;
pub mod memory;
pub mod opcode;
pub mod stack;
pub mod system;
mod host;

pub use opcode::{Instruction, OpCode, OPCODE_JUMPMAP};
