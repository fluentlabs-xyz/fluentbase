#![no_std]
#![allow(dead_code)]

extern crate alloc;
extern crate core;

pub use fluentbase_types::ExitCode;

// #[cfg(feature = "runtime")]
// pub mod api_runtime;
// #[cfg(not(feature = "runtime"))]
// pub mod api_system;
mod bindings;
mod macros;
