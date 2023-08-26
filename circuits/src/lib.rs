#![allow(
    dead_code,
    unreachable_patterns,
    unused_macros,
    clippy::too_many_arguments
)]
#![deny(unsafe_code)]

extern crate core;

mod constraint_builder;
mod fluentbase_circuit;
mod gadgets;
mod poseidon_circuit;
mod prover;
mod runtime_circuit;
mod rwasm_circuit;
#[cfg(test)]
mod testing;
mod unrolled_bytecode;
mod util;
