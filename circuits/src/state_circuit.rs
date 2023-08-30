///! This circuit is inspired by StateCircuit from PSE/Scroll. We kept only stack and memory ops,
/// additionally extended it with WASM structures, like global variables and function tables.
mod circuit;
mod lexicographic_ordering;
mod mpi_config;
mod param;
mod rw_row;
mod rw_table;
mod sort_keys;
mod tag;

pub use circuit::{StateCircuitConfig, StateLookup};
