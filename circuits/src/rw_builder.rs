pub mod copy_row;
mod opcode;
mod platform;
pub mod rw_row;

use crate::{
    exec_step::{ExecStep, GadgetError},
    rw_builder::{
        opcode::{
            build_consume_fuel_rw_ops,
            build_generic_rw_ops,
            build_memory_copy_rw_ops,
            build_memory_fill_rw_ops,
            build_table_fill_rw_ops,
        },
        platform::build_platform_rw_ops,
        rw_row::{RwRow, RwTableContextTag},
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
            Instruction::TableFill(table_idx) => {
                build_table_fill_rw_ops(step, table_idx.to_u32())?;
            }
            _ => {
                build_generic_rw_ops(step, step.instr().get_rw_ops())?;
            }
        }
        Ok(())
    }
}
