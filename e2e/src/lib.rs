#![allow(soft_unstable)]
#![feature(test)]

extern crate alloc;
extern crate core;

// #[cfg(test)]
// mod bench;
#[cfg(test)]
mod bridge;
#[cfg(test)]
mod constructor;
#[cfg(test)]
mod evm;
#[cfg(test)]
mod gas;
#[cfg(test)]
mod multicall;
#[cfg(test)]
mod nitro;
#[cfg(test)]
mod precompile;
#[cfg(test)]
mod router;
#[cfg(test)]
mod stateless;
#[cfg(test)]
mod utils;
#[cfg(test)]
mod wasm;
