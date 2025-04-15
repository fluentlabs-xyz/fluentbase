#![cfg_attr(not(feature = "std"), no_std)]
#![allow(dead_code)]
#![warn(unused_crate_dependencies)]

mod binary_format;
mod config;
mod context;
mod executor;
mod handler;
mod instr_ptr;
mod instruction_table;
mod memory;
mod module;
mod opcodes;
mod types;
mod utils;

extern crate alloc;
extern crate core;

pub use config::*;
pub use context::*;
pub use executor::*;
pub use handler::*;
pub use instruction_table::*;
pub use module::*;
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
