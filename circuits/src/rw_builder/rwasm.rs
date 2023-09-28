use crate::{
    exec_step::{ExecStep, GadgetError},
    rw_builder::{
        opcode::{build_stack_read_rw_ops, build_stack_write_rw_ops},
        rw_row::{RwRow, RwTableContextTag},
    },
};

pub fn build_rwasm_transact_rw_ops(step: &mut ExecStep) -> Result<(), GadgetError> {
    // read input
    let output_len = build_stack_read_rw_ops(step, 0)?;
    let output_offset = build_stack_read_rw_ops(step, 1)?;
    let input_len = build_stack_read_rw_ops(step, 2)?;
    let input_offset = build_stack_read_rw_ops(step, 3)?;
    let code_len = build_stack_read_rw_ops(step, 4)?;
    let code_offset = build_stack_read_rw_ops(step, 5)?;

    let mut input = vec![0; input_len.as_usize()];
    step.curr_read_memory(
        input_offset.as_u64(),
        input.as_mut_ptr(),
        input_len.as_u32(),
    )?;
    let mut bytecode = vec![0; code_len.as_usize()];
    step.curr_read_memory(
        code_offset.as_u64(),
        bytecode.as_mut_ptr(),
        code_len.as_u32(),
    )?;
    // TODO: "add memory read rw rows"
    step.rw_rows.push(RwRow::Context {
        rw_counter: step.next_rw_counter(),
        is_write: true,
        call_id: step.call_id,
        tag: RwTableContextTag::CallDepth,
        value: step.call_id as u64,
        prev_value: 0, // FIXME
    });
    // TODO: "add memory write rw rows"
    build_stack_write_rw_ops(step, 0)?;
    // increase call id
    step.call_id += 1;
    Ok(())
}
