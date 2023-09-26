pub mod copy_row;
mod opcode;
mod platform;
pub mod rw_row;
mod rwasm;
mod wasi;

use crate::{
    exec_step::{ExecStep, GadgetError},
    rw_builder::{
        opcode::{
            build_consume_fuel_rw_ops,
            build_generic_rw_ops,
            build_memory_copy_rw_ops,
            build_memory_fill_rw_ops,
        },
        platform::{build_sys_halt_rw_ops, build_sys_read_rw_ops, build_sys_write_rw_ops},
        rw_row::{RwRow, RwTableContextTag},
        rwasm::build_rwasm_transact_rw_ops,
        wasi::{
            build_wasi_args_get_rw_ops,
            build_wasi_args_sizes_get_rw_ops,
            build_wasi_environ_get_rw_ops,
            build_wasi_environ_sizes_get_rw_ops,
            build_wasi_fd_write_rw_ops,
            build_wasi_proc_exit_rw_ops,
        },
    },
};
use fluentbase_runtime::SysFuncIdx;
use fluentbase_rwasm::engine::bytecode::Instruction;

#[derive(Default)]
pub struct RwBuilder {}

impl RwBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn build(&mut self, step: &mut ExecStep) -> Result<(), GadgetError> {
        // we must do all context lookups in the beginning, otherwise copy lookup might break it
        step.rw_rows.push(RwRow::Context {
            rw_counter: step.next_rw_counter(),
            is_write: true,
            call_id: step.call_id,
            tag: RwTableContextTag::ProgramCounter,
            value: step.pc_diff(),
        });
        // step.rw_rows.push(RwRow::Context {
        //     rw_counter: step.next_rw_counter(),
        //     is_write: true,
        //     call_id: step.call_id,
        //     tag: RwTableContextTag::MemorySize,
        //     value: step.curr().memory_size as u64,
        // });
        // step.rw_rows.push(RwRow::Context {
        //     rw_counter: step.next_rw_counter(),
        //     is_write: true,
        //     call_id: step.call_id,
        //     tag: RwTableContextTag::StackPointer,
        //     value: step.stack_len() as u64,
        // });
        match step.instr() {
            Instruction::Call(fn_idx) => {
                build_platform_rw_ops(step, SysFuncIdx::from(*fn_idx))?;
            }
            Instruction::ConsumeFuel(_) => {
                build_consume_fuel_rw_ops(step)?;
            }
            Instruction::MemoryCopy => {
                build_memory_copy_rw_ops(step)?;
            }
            Instruction::MemoryFill => {
                build_memory_fill_rw_ops(step)?;
            }
            _ => {
                build_generic_rw_ops(step, step.instr().get_rw_ops())?;
            }
        }
        Ok(())
    }
}

fn build_platform_rw_ops(step: &mut ExecStep, sys_func: SysFuncIdx) -> Result<(), GadgetError> {
    match sys_func {
        // rwasm calls
        SysFuncIdx::RWASM_TRANSACT => build_rwasm_transact_rw_ops(step),
        // sys calls
        SysFuncIdx::SYS_HALT => build_sys_halt_rw_ops(step),
        SysFuncIdx::SYS_WRITE => build_sys_write_rw_ops(step),
        SysFuncIdx::SYS_READ => build_sys_read_rw_ops(step),
        // wasi calls
        SysFuncIdx::WASI_PROC_EXIT => build_wasi_proc_exit_rw_ops(step),
        SysFuncIdx::WASI_FD_WRITE => build_wasi_fd_write_rw_ops(step),
        SysFuncIdx::WASI_ENVIRON_SIZES_GET => build_wasi_environ_sizes_get_rw_ops(step),
        SysFuncIdx::WASI_ENVIRON_GET => build_wasi_environ_get_rw_ops(step),
        SysFuncIdx::WASI_ARGS_SIZES_GET => build_wasi_args_sizes_get_rw_ops(step),
        SysFuncIdx::WASI_ARGS_GET => build_wasi_args_get_rw_ops(step),
        // this is not possible right now
        _ => Err(GadgetError::UnknownSysCall(sys_func)),
    }
}
