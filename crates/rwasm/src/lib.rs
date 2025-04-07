#![cfg_attr(not(feature = "std"), no_std)]
#![allow(dead_code)]
#![warn(unused_crate_dependencies)]

mod config;
mod context;
mod executor;
mod handler;
mod opcodes;
mod types;
mod utils;

extern crate alloc;
extern crate core;

pub use config::*;
pub use context::*;
pub use executor::*;
pub use handler::*;
pub use rwasm::{
    core::{HostError, TrapCode},
    engine::{bytecode::Instruction, stack::ValueStackPtr, RwasmConfig, StateRouterConfig},
    memory::MemoryEntity,
    rwasm::{
        instruction::InstructionExtra,
        BinaryFormat,
        InstructionSet,
        RwasmModule,
        RwasmModuleInstance,
    },
};
pub use types::*;
