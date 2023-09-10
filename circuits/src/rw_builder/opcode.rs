use crate::{
    exec_step::{ExecStep, GadgetError},
    rw_builder::rw_row::RwRow,
};
use fluentbase_rwasm::RwOp;

pub fn build_stack_write_rw_ops(
    step: &mut ExecStep,
    local_depth: usize,
) -> Result<(), GadgetError> {
    let addr = step.next_nth_stack_addr(local_depth)?;
    let value = step.next_nth_stack_value(local_depth)?;
    step.rw_rows.push(RwRow::Stack {
        rw_counter: step.next_rw_counter(),
        is_write: true,
        call_id: step.call_id,
        stack_pointer: addr as usize,
        value,
    });
    Ok(())
}

pub fn build_stack_read_rw_ops(step: &mut ExecStep, local_depth: usize) -> Result<(), GadgetError> {
    let addr = step.curr_nth_stack_addr(local_depth)?;
    let value = step.curr_nth_stack_value(local_depth)?;
    step.rw_rows.push(RwRow::Stack {
        rw_counter: step.next_rw_counter(),
        is_write: false,
        call_id: step.call_id,
        stack_pointer: addr as usize,
        value,
    });
    Ok(())
}

pub fn build_global_write_rw_ops(
    step: &mut ExecStep,
    global_index: usize,
) -> Result<(), GadgetError> {
    let value = step.curr_nth_stack_value(0)?;
    step.rw_rows.push(RwRow::Global {
        rw_counter: step.next_rw_counter(),
        is_write: true,
        call_id: step.call_id,
        global_index,
        value,
    });
    Ok(())
}

pub fn build_global_read_rw_ops(
    step: &mut ExecStep,
    global_index: usize,
) -> Result<(), GadgetError> {
    let value = step.next_nth_stack_value(0)?;
    step.rw_rows.push(RwRow::Global {
        rw_counter: step.next_rw_counter(),
        is_write: false,
        call_id: step.call_id,
        global_index,
        value,
    });
    Ok(())
}

pub fn build_memory_write_rw_ops(
    step: &mut ExecStep,
    offset: u32,
    length: u32,
) -> Result<(), GadgetError> {
    let value = step.curr_nth_stack_value(0)?;
    let addr = step.curr_nth_stack_value(1)?;
    let value_le_bytes = value.to_bits().to_le_bytes();
    (0..length).for_each(|i| {
        step.rw_rows.push(RwRow::Memory {
            rw_counter: step.next_rw_counter(),
            is_write: true,
            call_id: step.call_id,
            memory_address: addr.as_u64() + (offset + i) as u64,
            value: value_le_bytes[i as usize],
            length,
            signed: false,
        });
    });
    Ok(())
}

pub fn build_memory_read_rw_ops(
    step: &mut ExecStep,
    offset: u32,
    length: u32,
    signed: bool,
) -> Result<(), GadgetError> {
    let addr = step.curr_nth_stack_value(0)?;
    let mut value_le_bytes = vec![0; length as usize];
    let mem_addr = offset as u64 + addr.as_u64();
    step.curr_read_memory(mem_addr, value_le_bytes.as_mut_ptr(), length)?;
    (0..length).for_each(|i| {
        step.rw_rows.push(RwRow::Memory {
            rw_counter: step.next_rw_counter(),
            is_write: false,
            call_id: step.call_id,
            memory_address: mem_addr + i as u64,
            value: value_le_bytes[i as usize],
            length,
            signed,
        });
    });
    Ok(())
}

pub fn build_table_size_read_rw_ops(
    step: &mut ExecStep,
    table_idx: u32,
) -> Result<(), GadgetError> {
    let table_size = step.read_table_size(table_idx);
    step.rw_rows.push(RwRow::Table {
        rw_counter: step.next_rw_counter(),
        is_write: false,
        call_id: step.call_id,
        address: (table_idx * 1024) as u64,
        value: table_size as u64,
    });
    Ok(())
}

pub fn build_table_size_write_rw_ops(
    step: &mut ExecStep,
    table_idx: u32,
) -> Result<(), GadgetError> {
    let table_size = step.read_table_size(table_idx);
    let grow = step.curr_nth_stack_value(1)?;
    step.rw_rows.push(RwRow::Table {
        rw_counter: step.next_rw_counter(),
        is_write: true,
        call_id: step.call_id,
        address: (table_idx * 1024) as u64,
        value: (table_size as u32 + grow.as_u32()) as u64,
    });
    Ok(())
}

pub fn build_table_elem_read_rw_ops(
    step: &mut ExecStep,
    table_idx: u32,
) -> Result<(), GadgetError> {
    let elem_index = step.curr_nth_stack_value(0)?;
    let value = step.next_nth_stack_value(0)?;
    step.rw_rows.push(RwRow::Table {
        rw_counter: step.next_rw_counter(),
        is_write: false,
        call_id: step.call_id,
        address: (table_idx * 1024) as u64 + elem_index.as_u32() as u64 + 1,
        value: value.as_u32() as u64,
    });
    Ok(())
}

pub fn build_table_elem_write_rw_ops(
    step: &mut ExecStep,
    table_idx: u32,
) -> Result<(), GadgetError> {
    let elem_index = step.curr_nth_stack_value(1)?;
    let value = step.curr_nth_stack_value(2)?;
    step.rw_rows.push(RwRow::Table {
        rw_counter: step.next_rw_counter(),
        is_write: true,
        call_id: step.call_id,
        address: (table_idx * 1024) as u64 + elem_index.as_u32() as u64 + 1,
        value: value.as_u32() as u64,
    });
    Ok(())
}

pub fn build_generic_rw_ops(step: &mut ExecStep, rw_ops: Vec<RwOp>) -> Result<(), GadgetError> {
    let mut stack_reads = 0;
    let mut stack_writes = 0;
    for rw_op in rw_ops {
        match rw_op {
            RwOp::StackWrite(local_depth) => {
                build_stack_write_rw_ops(step, stack_writes + local_depth as usize)?;
                stack_writes += 1
            }
            RwOp::StackRead(local_depth) => {
                build_stack_read_rw_ops(step, stack_reads + local_depth as usize)?;
                stack_reads += 1;
            }
            RwOp::GlobalWrite(global_index) => {
                build_global_write_rw_ops(step, global_index as usize)?;
            }
            RwOp::GlobalRead(global_index) => {
                build_global_read_rw_ops(step, global_index as usize)?;
            }
            RwOp::MemoryWrite { offset, length, .. } => {
                build_memory_write_rw_ops(step, offset, length)?;
            }
            RwOp::MemoryRead {
                offset,
                length,
                signed,
            } => {
                build_memory_read_rw_ops(step, offset, length, signed)?;
            }
            RwOp::TableSizeRead(table_idx) => {
                build_table_size_read_rw_ops(step, table_idx)?;
            }
            RwOp::TableSizeWrite(table_idx) => {
                build_table_size_write_rw_ops(step, table_idx)?;
            }
            RwOp::TableElemRead(table_idx) => {
                build_table_elem_read_rw_ops(step, table_idx)?;
            }
            RwOp::TableElemWrite(table_idx) => {
                build_table_elem_write_rw_ops(step, table_idx)?;
            }
            _ => unreachable!("rw ops mapper is not implemented {:?}", rw_op),
        }
    }
    Ok(())
}
