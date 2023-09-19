#![allow(
    dead_code,
    unreachable_patterns,
    unused_macros,
    clippy::too_many_arguments,
    clippy::type_complexity
)]

#![feature(type_name_of_val)]
#![feature(associated_type_defaults)]

extern crate core;

mod bitwise_check;
mod constraint_builder;
mod copy_circuit;
mod exec_step;
mod fixed_table;
mod fluentbase_circuit;
mod gadgets;
mod lookup_table;
mod pi_circuit;
mod poseidon_circuit;
mod prover;
mod range_check;
mod runtime_circuit;
mod rw_builder;
mod rwasm_circuit;
mod state_circuit;
#[cfg(test)]
mod testing;
mod util;
mod witness;
