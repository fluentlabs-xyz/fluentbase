pub mod copy_row;
mod opcode;
mod platform;
pub mod rw_row;

use crate::{
    exec_step::{ExecStep, GadgetError},
    rw_builder::{
        opcode::{build_generic_rw_ops, build_memory_copy_rw_ops},
        platform::build_platform_rw_ops,
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
        match step.instr() {
            Instruction::Call(fn_idx) => {
                let sys_func = SysFuncIdx::from(*fn_idx);
                build_platform_rw_ops(step, sys_func)?;
            }
            Instruction::MemoryCopy => {
                build_memory_copy_rw_ops(step)?;
            }
            _ => {
                build_generic_rw_ops(step, step.instr().get_rw_ops())?;
            }
        }
        Ok(())
    }
}
