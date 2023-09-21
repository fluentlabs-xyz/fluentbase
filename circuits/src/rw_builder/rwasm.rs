use crate::{
    exec_step::{ExecStep, GadgetError},
    rw_builder::opcode::build_stack_read_rw_ops,
};

pub fn build_rwasm_transact_rw_ops(step: &mut ExecStep) -> Result<(), GadgetError> {
    // read error code
    build_stack_read_rw_ops(step, 0)?;
    Ok(())
}
