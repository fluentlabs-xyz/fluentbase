#![no_std]

extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;

use alloc::string::ToString;

pub mod compiler;

#[cfg(test)]
mod compiler_tests;
pub(crate) mod consts;
#[macro_use]
pub mod macros;
pub mod primitives;

pub mod translator;
pub mod utilities;

#[cfg(test)]
#[ctor::ctor]
fn log_init() {
    let init_res =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
            .try_init();
    if let Err(e) = init_res {
        panic!("failed to init logger: {}", e.to_string());
    }
}
