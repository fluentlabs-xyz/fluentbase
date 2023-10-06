use crate::{
    exec_step::{ExecStep, GadgetError},
    rw_builder::{
        copy_row::{CopyRow, CopyTableTag},
        opcode::{build_stack_read_rw_ops, build_stack_write_rw_ops},
        rw_row::RwRow,
    },
};

pub fn build_wasi_proc_exit_rw_ops(step: &mut ExecStep) -> Result<(), GadgetError> {
    // read error code
    build_stack_read_rw_ops(step, 0)?;
    Ok(())
}

pub fn build_wasi_fd_write_rw_ops(step: &mut ExecStep) -> Result<(), GadgetError> {
    // write error code (its always ERRNO_CANCELLED)
    build_stack_write_rw_ops(step, 0)?;
    Ok(())
}

pub fn build_wasi_environ_sizes_get_rw_ops(step: &mut ExecStep) -> Result<(), GadgetError> {
    // write error code (its always ERRNO_CANCELLED)
    build_stack_write_rw_ops(step, 0)?;
    Ok(())
}

pub fn build_wasi_environ_get_rw_ops(step: &mut ExecStep) -> Result<(), GadgetError> {
    // write error code (its always ERRNO_CANCELLED)
    build_stack_write_rw_ops(step, 0)?;
    Ok(())
}

pub fn build_wasi_args_sizes_get_rw_ops(step: &mut ExecStep) -> Result<(), GadgetError> {
    // read rp0 and rp1 addresses
    let rp1_offset = build_stack_read_rw_ops(step, 0)?;
    let rp0_offset = build_stack_read_rw_ops(step, 1)?;
    // get ro0 and rw1 values
    assert_eq!(step.next_trace.clone().unwrap().memory_changes[0].len, 8);
    let data = step.next_trace.clone().unwrap().memory_changes[0]
        .data
        .clone();
    let data = data.as_slice();
    // write rp0 into memory
    data[0..4].iter().enumerate().for_each(|(i, value)| {
        step.rw_rows.push(RwRow::Memory {
            rw_counter: step.next_rw_counter(),
            is_write: true,
            call_id: step.call_id,
            memory_address: rp0_offset.as_u64() + i as u64,
            value: *value,
            signed: false,
        });
    });
    // write rp1 into memory
    data[4..8].iter().enumerate().for_each(|(i, value)| {
        step.rw_rows.push(RwRow::Memory {
            rw_counter: step.next_rw_counter(),
            is_write: true,
            call_id: step.call_id,
            memory_address: rp1_offset.as_u64() + i as u64,
            value: *value,
            signed: false,
        });
    });
    // write result on stack (always zero)
    build_stack_write_rw_ops(step, 0)?;
    Ok(())
}

pub fn build_wasi_args_get_rw_ops(step: &mut ExecStep) -> Result<(), GadgetError> {
    // read argv and argv_buffer offsets from the stack
    let argv_buffer_offset = build_stack_read_rw_ops(step, 0)?;
    let argv_offset = build_stack_read_rw_ops(step, 1)?;
    // extract input from memory changes
    let input_data = step.next()?.memory_changes[0].data.clone();
    let argv_data = step.next()?.memory_changes[1].data.clone();
    // copy entire input to the memory
    let copy_rw_counter = step.next_rw_counter();
    input_data
        .as_slice()
        .iter()
        .enumerate()
        .for_each(|(i, value)| {
            step.rw_rows.push(RwRow::Memory {
                rw_counter: step.next_rw_counter(),
                is_write: true,
                call_id: step.call_id,
                memory_address: argv_buffer_offset.as_u64() + i as u64,
                value: *value,
                signed: false,
            });
        });
    // copy 1 to argv offset value
    argv_data
        .as_slice()
        .iter()
        .enumerate()
        .for_each(|(i, value)| {
            step.rw_rows.push(RwRow::Memory {
                rw_counter: step.next_rw_counter(),
                is_write: true,
                call_id: step.call_id,
                memory_address: argv_offset.as_u64() + i as u64,
                value: *value,
                signed: false,
            });
        });
    step.copy_rows.push(CopyRow {
        tag: CopyTableTag::ReadInput,
        from_address: 0u32,
        to_address: argv_buffer_offset.as_u32(),
        length: input_data.len() as u32,
        rw_counter: copy_rw_counter,
        data: input_data.to_vec().clone(),
    });
    Ok(())
}
