#![cfg_attr(not(feature = "std"), no_std)]
// #![warn(unused_crate_dependencies)]

extern crate alloc;
extern crate core;
extern crate solana_rbpf;

use fluentbase_sdk::ExitCode;

pub mod blended;
pub mod helpers;
// pub mod helpers_svm;
pub mod svm_core;
pub mod types;

// use fluentbase_sdk::{
//     journal::{JournalState, JournalStateBuilder},
//     Address,
//     ContractContext,
//     SharedAPI,
//     U256,
// };
use solana_ee_core::{
    context,
    context::ExecContextObject,
    helpers::{
        serialize_parameters_aligned,
        SyscallAbort,
        SyscallLog,
        SyscallMemcpy,
        SyscallStubInterceptor,
        INSTRUCTION_METER_BUDGET,
    },
};
use solana_program::{account_info::AccountInfo, clock::Epoch, pubkey::Pubkey};
use solana_rbpf::{
    ebpf,
    elf::Executable,
    error::{EbpfError, ProgramResult},
    memory_region::MemoryRegion,
    program::{BuiltinFunction, BuiltinProgram, FunctionRegistry},
    verifier::RequisiteVerifier,
    vm::Config,
};
