#![allow(soft_unstable)]
#![feature(test)]

extern crate alloc;
extern crate core;

#[cfg(test)]
mod evm;
#[cfg(test)]
mod wasm;

#[cfg(test)]
mod bench;
mod bridge;
mod gas;
mod genesis;
mod router;
mod stateless;
mod utils;
