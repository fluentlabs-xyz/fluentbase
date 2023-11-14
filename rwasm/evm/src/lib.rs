extern crate alloc;

pub(crate) const USE_GAS: bool = !cfg!(feature = "no_gas_measuring");

pub mod compiler;

pub mod primitives;
pub mod interpreter;
pub mod macros;
#[cfg(test)]
mod compiler_tests;
