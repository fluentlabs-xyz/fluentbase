#![allow(
    dead_code,
    unreachable_patterns,
    unused_macros,
    clippy::too_many_arguments,
    clippy::type_complexity
)]

extern crate core;

mod constraint_builder;
mod copy_circuit;
mod fixed_table;
mod fluentbase_circuit;
mod gadgets;
mod lookup_table;
mod pi_circuit;
mod poseidon_circuit;
mod prover;
mod range_check;
mod runtime_circuit;
mod rwasm_circuit;
mod state_circuit;
#[cfg(test)]
mod testing;
mod trace_step;
mod types;
mod util;
mod witness;
