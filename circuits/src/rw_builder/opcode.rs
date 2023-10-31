use crate::{
    exec_step::{ExecStep, GadgetError, MAX_TABLE_SIZE},
    rw_builder::{
        copy_row::{CopyRow, CopyTableTag},
        rw_row::{RwRow, RwTableContextTag},
    },
};
use fluentbase_rwasm::{common::UntypedValue, RwOp};

pub fn build_stack_write_rw_ops(
    step: &mut ExecStep,
    local_depth: usize,
) -> Result<UntypedValue, GadgetError> {
    let addr = step.next_nth_stack_addr(local_depth)?;
    let value = step.next_nth_stack_value(local_depth)?;
    step.rw_rows.push(RwRow::Stack {
        rw_counter: step.next_rw_counter(),
        is_write: true,
        call_id: step.call_id,
        stack_pointer: addr as usize,
        value,
    });
    Ok(value)
}

pub fn build_stack_read_rw_ops(
    step: &mut ExecStep,
    local_depth: usize,
) -> Result<UntypedValue, GadgetError> {
    let addr = step.curr_nth_stack_addr(local_depth)?;
    let value = step.curr_nth_stack_value(local_depth)?;
    step.rw_rows.push(RwRow::Stack {
        rw_counter: step.next_rw_counter(),
        is_write: false,
        call_id: step.call_id,
        stack_pointer: addr as usize,
        value,
    });
    Ok(value)
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
            signed,
        });
    });
    Ok(())
}

pub fn build_memory_size_write_rw_ops(step: &mut ExecStep) -> Result<(), GadgetError> {
    step.rw_rows.push(RwRow::Context {
        rw_counter: step.next_rw_counter(),
        is_write: true,
        call_id: step.call_id,
        tag: RwTableContextTag::MemorySize,
        value: step.next().map(|t| t.memory_size).unwrap_or_default() as u64,
    });
    Ok(())
}

pub fn build_memory_size_read_rw_ops(step: &mut ExecStep) -> Result<(), GadgetError> {
    step.rw_rows.push(RwRow::Context {
        rw_counter: step.next_rw_counter(),
        is_write: false,
        call_id: step.call_id,
        tag: RwTableContextTag::MemorySize,
        value: step.curr().memory_size as u64,
    });
    Ok(())
}

pub fn build_table_size_read_rw_ops(
    step: &mut ExecStep,
    table_index: u32,
) -> Result<(), GadgetError> {
    let table_size = step.read_table_size(table_index);
    step.rw_rows.push(RwRow::Context {
        rw_counter: step.next_rw_counter(),
        is_write: false,
        call_id: step.call_id,
        tag: RwTableContextTag::TableSize(table_index),
        value: table_size as u64,
    });
    Ok(())
}

pub fn build_table_size_write_rw_ops(
    step: &mut ExecStep,
    table_index: u32,
) -> Result<(), GadgetError> {
    let table_size = step.read_table_size(table_index);
    let table_diff: u32 = step
        .next()?
        .table_size_changes
        .iter()
        .filter(|change| change.table_idx == table_index)
        .map(|v| v.delta)
        .sum();
    step.rw_rows.push(RwRow::Context {
        rw_counter: step.next_rw_counter(),
        is_write: true,
        call_id: step.call_id,
        tag: RwTableContextTag::TableSize(table_index),
        value: (table_size as u32 + table_diff) as u64,
    });
    Ok(())
}

pub fn build_table_get_rw_ops(
    step: &mut ExecStep,
    table_idx: u32,
) -> Result<(), GadgetError> {
    let table_size = step.read_table_size(table_idx);
    let elem_index = step.curr_nth_stack_value(0)?;
    let addr = step.curr_nth_stack_addr(0)?;
    step.rw_rows.push(RwRow::Stack {
        rw_counter: step.next_rw_counter(),
        is_write: false,
        call_id: step.call_id,
        stack_pointer: addr as usize,
        value: elem_index,
    });
    if elem_index < table_size.into() {
        let addr = step.next_nth_stack_addr(0)?;
        let value = step.next_nth_stack_value(0)?;
        build_table_elem_read_rw_ops(step, table_idx)?;
        step.rw_rows.push(RwRow::Stack {
            rw_counter: step.next_rw_counter(),
            is_write: true,
            call_id: step.call_id,
            stack_pointer: addr as usize,
            value,
        });
    }
    build_table_size_read_rw_ops(step, table_idx)?;
    Ok(())
}

pub fn build_table_elem_read_rw_ops(
    step: &mut ExecStep,
    table_idx: u32,
) -> Result<(), GadgetError> {

    let table_size = step.read_table_size(table_idx);
    let elem_index = step.curr_nth_stack_value(0)?;
    let value = step.next_nth_stack_value(0)?;
    step.rw_rows.push(RwRow::Table {
        rw_counter: step.next_rw_counter(),
        is_write: false,
        call_id: step.call_id,
        address: (table_idx * (MAX_TABLE_SIZE as u32)) as u64 + elem_index.as_u32() as u64,
        value: value.as_u32() as u64,
    });
    Ok(())
}

pub fn build_table_elem_write_rw_ops(
    step: &mut ExecStep,
    table_idx: u32,
) -> Result<(), GadgetError> {
    let value = step.curr_nth_stack_value(0)?;
    let elem_index = step.curr_nth_stack_value(1)?;
    step.rw_rows.push(RwRow::Table {
        rw_counter: step.next_rw_counter(),
        is_write: true,
        call_id: step.call_id,
        address: (table_idx * (MAX_TABLE_SIZE as u32)) as u64 + elem_index.as_u32() as u64,
        value: value.as_u32() as u64,
    });
    Ok(())
}

pub fn build_memory_copy_rw_ops(step: &mut ExecStep) -> Result<(), GadgetError> {
    // pop 3 elems from stack
    let len = build_stack_read_rw_ops(step, 0)?;
    let source = build_stack_read_rw_ops(step, 1)?;
    let dest = build_stack_read_rw_ops(step, 2)?;
    // read copied data
    let mut data = vec![0; len.as_u32() as usize];
    step.curr_read_memory(source.as_u64(), data.as_mut_ptr(), len.as_u32())?;
    let copy_rw_counter = step.next_rw_counter();
    // read result to the memory
    data.iter().enumerate().for_each(|(i, value)| {
        step.rw_rows.push(RwRow::Memory {
            rw_counter: step.next_rw_counter(),
            is_write: false,
            call_id: step.call_id,
            memory_address: source.as_u64() + i as u64,
            value: *value,
            signed: false,
        });
    });
    // write result to the memory
    data.iter().enumerate().for_each(|(i, value)| {
        step.rw_rows.push(RwRow::Memory {
            rw_counter: step.next_rw_counter(),
            is_write: true,
            call_id: step.call_id,
            memory_address: dest.as_u64() + i as u64,
            value: *value,
            signed: false,
        });
    });
    // create copy row
    step.copy_rows.push(CopyRow {
        tag: CopyTableTag::CopyMemory,
        from_address: source.as_u32(),
        to_address: dest.as_u32(),
        length: len.as_u32(),
        rw_counter: copy_rw_counter,
        data: vec![0; len.as_usize()],
    });
    Ok(())
}

pub fn build_memory_fill_rw_ops(step: &mut ExecStep) -> Result<(), GadgetError> {
    // pop 3 elems from stack
    let len = build_stack_read_rw_ops(step, 0)?;
    let value = build_stack_read_rw_ops(step, 1)?.as_u32() as u8;
    let dest = build_stack_read_rw_ops(step, 2)?;
    // remember rw counter before fill
    let fill_rw_counter = step.next_rw_counter();
    // read result to the memory
    (0..len.as_usize()).for_each(|i| {
        step.rw_rows.push(RwRow::Memory {
            rw_counter: step.next_rw_counter(),
            is_write: true,
            call_id: step.call_id,
            memory_address: dest.as_u64() + i as u64,
            value,
            signed: false,
        });
    });
    // create copy row
    step.copy_rows.push(CopyRow {
        tag: CopyTableTag::FillMemory,
        from_address: value as u32,
        to_address: dest.as_u32(),
        length: len.as_u32(),
        rw_counter: fill_rw_counter,
        data: vec![value as u32; len.as_usize()],
    });
    Ok(())
}

pub fn build_table_fill_rw_ops(step: &mut ExecStep, table_index: u32) -> Result<(), GadgetError> {
    // pop 3 elems from stack
    let range = build_stack_read_rw_ops(step, 0)?;
    let value = build_stack_read_rw_ops(step, 1)?;
    let start = build_stack_read_rw_ops(step, 2)?;
    build_table_fill_rw_ops_with_args(
        step,
        table_index,
        range.as_u32(),
        value.as_u32(),
        start.as_u32(),
    )
}

pub fn build_table_grow_rw_ops(step: &mut ExecStep, table_index: u32) -> Result<(), GadgetError> {
    // pop 3 elems from stack
    let delta = build_stack_read_rw_ops(step, 0)?;
    let init = build_stack_read_rw_ops(step, 1)?;
    // put result on stack
    let result = build_stack_write_rw_ops(step, 0)?;
    if result.as_u32() == u32::MAX {
        // return Ok(());
    }
    // fetch current table size
    let table_size = step.read_table_size(table_index) as u32;
    assert_eq!(table_size, result.as_u32());
    step.rw_rows.push(RwRow::Context {
        rw_counter: step.next_rw_counter(),
        is_write: true,
        call_id: step.call_id,
        tag: RwTableContextTag::TableSize(table_index),
        value: (table_size + delta.as_u32()) as u64,
    });
    // remember rw counter before fill
    let copy_rw_counter = step.next_rw_counter();
    // read result to the memory
    (0..delta.as_usize()).for_each(|i| {
        step.rw_rows.push(RwRow::Table {
            rw_counter: step.next_rw_counter(),
            is_write: true,
            call_id: step.call_id,
            address: (table_index * (MAX_TABLE_SIZE as u32)) as u64 + table_size as u64 + i as u64,
            value: init.as_u64(),
        });
    });
    // create copy row
    step.copy_rows.push(CopyRow {
        tag: CopyTableTag::FillTable,
        from_address: 0,
        to_address: table_index * (MAX_TABLE_SIZE as u32) + table_size,
        length: delta.as_u32(),
        rw_counter: copy_rw_counter,
        data: vec![init.as_u32(); delta.as_usize()],
    });
    Ok(())
}

pub fn build_table_fill_rw_ops_with_args(
    step: &mut ExecStep,
    table_index: u32,
    range: u32,
    value: u32,
    start: u32,
) -> Result<(), GadgetError> {
    println!("DEBUG BUILD VALUE {:#?}", value);
    build_table_size_read_rw_ops(step, table_index)?;
    // remember rw counter before fill
    let fill_rw_counter = step.next_rw_counter();
    // read result to the table
    (start as usize..(start as usize + range as usize)).for_each(|i| {
        step.rw_rows.push(RwRow::Table {
            rw_counter: step.next_rw_counter(),
            is_write: true,
            call_id: step.call_id,
            address: table_index as u64 * 1024 + i as u64,
            value: value as u64,
        });
    });
    // create copy row
    let row = CopyRow {
        tag: CopyTableTag::FillTable,
        from_address: value,
        to_address: table_index * 1024 + start,
        length: range,
        rw_counter: fill_rw_counter,
        data: vec![value; range as usize],
    };
    println!("DEBUG ROW {:#?}, TAG {}", row, row.tag as u32);
    step.copy_rows.push(row);
    Ok(())
}

pub fn build_table_copy_rw_ops(
    step: &mut ExecStep,
    table_dst: u32,
) -> Result<(), GadgetError> {
    //let table_src = step.next().unwrap().opcode.aux_value().unwrap_or_default().as_u32();
    let table_src = step.curr().next_table_idx.unwrap().to_u32();
    // pop 3 elems from stack
    build_table_size_read_rw_ops(step, table_dst)?;
    build_table_size_read_rw_ops(step, table_src)?;
    let length = build_stack_read_rw_ops(step, 0)?;
    let src_eidx = build_stack_read_rw_ops(step, 1)?;
    let dst_eidx = build_stack_read_rw_ops(step, 2)?;
    // read copied data
    let mut data = vec![0; length.as_u32() as usize];
    for i in 0..length.as_usize() {
        data[i] = step.read_table_elem(table_src, i as u32).unwrap().as_u32();
    }
    let copy_rw_counter = step.next_rw_counter();
    // read result to the table
    data.iter().enumerate().for_each(|(i, value)| {
        step.rw_rows.push(RwRow::Table {
            rw_counter: step.next_rw_counter(),
            is_write: false,
            call_id: step.call_id,
            address: table_src as u64 * 1024 + src_eidx.as_u64() + i as u64,
            value: *value as u64,
        });
    });
    // write result to the table
    data.iter().enumerate().for_each(|(i, value)| {
        step.rw_rows.push(RwRow::Table {
            rw_counter: step.next_rw_counter(),
            is_write: true,
            call_id: step.call_id,
            address: table_dst as u64 * 1024 + dst_eidx.as_u64() + i as u64,
            value: *value as u64,
        });
    });
    // create copy row
    step.copy_rows.push(CopyRow {
        tag: CopyTableTag::CopyTable,
        from_address: table_src * 1024 + src_eidx.as_u32(),
        to_address: table_dst * 1024 + dst_eidx.as_u32(),
        length: length.as_u32(),
        rw_counter: copy_rw_counter,
        data: vec![0; length.as_usize()],
    });
    Ok(())
}

pub fn build_consume_fuel_rw_ops(step: &mut ExecStep) -> Result<(), GadgetError> {
    let consumed_fuel = step.curr().consumed_fuel;
    let fuel = step.instr().aux_value().unwrap_or_default();
    step.rw_rows.push(RwRow::Context {
        rw_counter: step.next_rw_counter(),
        is_write: true,
        call_id: step.call_id,
        tag: RwTableContextTag::ConsumedFuel,
        value: consumed_fuel + fuel.as_u64(),
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
            RwOp::MemorySizeWrite => {
                build_memory_size_write_rw_ops(step)?;
            }
            RwOp::MemorySizeRead => {
                build_memory_size_read_rw_ops(step)?;
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
