pub mod copy_row;
pub mod rw_row;

use crate::{
    exec_step::{ExecStep, GadgetError},
    rw_builder::{
        copy_row::{CopyRow, CopyTableTag},
        rw_row::RwRow,
    },
};
use fluentbase_runtime::SysFuncIdx;
use fluentbase_rwasm::{engine::bytecode::Instruction, RwOp};

#[derive(Default)]
pub struct RwBuilder {
    call_id: usize,
}

impl RwBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn build(&mut self, step: &mut ExecStep) -> Result<(), GadgetError> {
        match step.instr() {
            Instruction::Call(fn_idx) => {
                let sys_func = SysFuncIdx::from(*fn_idx);
                self.build_sys_call(step, sys_func)?;
            }
            _ => {
                self.build_rw_ops(step, step.instr().get_rw_ops())?;
            }
        }
        Ok(())
    }

    fn build_rw_ops(&mut self, step: &mut ExecStep, rw_ops: Vec<RwOp>) -> Result<(), GadgetError> {
        let mut stack_reads = 0;
        let mut stack_writes = 0;
        for rw_op in rw_ops {
            let rw_counter = step.next_rw_counter();
            let call_id = self.call_id;
            match rw_op {
                RwOp::StackWrite(local_depth) => {
                    let addr = step.next_nth_stack_addr(stack_writes + local_depth as usize)?;
                    let value = step.next_nth_stack_value(stack_writes + local_depth as usize)?;
                    step.rw_rows.push(RwRow::Stack {
                        rw_counter,
                        is_write: true,
                        call_id,
                        stack_pointer: addr as usize,
                        value,
                    });
                    stack_writes += 1
                }
                RwOp::StackRead(local_depth) => {
                    let addr = step.curr_nth_stack_addr(stack_reads + local_depth as usize)?;
                    let value = step.curr_nth_stack_value(stack_reads + local_depth as usize)?;
                    step.rw_rows.push(RwRow::Stack {
                        rw_counter,
                        is_write: false,
                        call_id,
                        stack_pointer: addr as usize,
                        value,
                    });
                    stack_reads += 1;
                }
                RwOp::GlobalWrite(global_index) => {
                    let value = step.curr_nth_stack_value(0)?;
                    step.rw_rows.push(RwRow::Global {
                        rw_counter,
                        is_write: true,
                        call_id,
                        global_index: global_index as usize,
                        value,
                    });
                }
                RwOp::GlobalRead(global_index) => {
                    let value = step.next_nth_stack_value(0)?;
                    step.rw_rows.push(RwRow::Global {
                        rw_counter,
                        is_write: false,
                        call_id,
                        global_index: global_index as usize,
                        value,
                    });
                }
                RwOp::MemoryWrite { offset, length, .. } => {
                    let value = step.curr_nth_stack_value(0)?;
                    let addr = step.curr_nth_stack_value(1)?;
                    let value_le_bytes = value.to_bits().to_le_bytes();
                    (0..length).for_each(|i| {
                        step.rw_rows.push(RwRow::Memory {
                            rw_counter,
                            is_write: true,
                            call_id,
                            memory_address: addr.as_u64() + offset as u64 + i as u64,
                            value: value_le_bytes[i as usize],
                            length,
                            signed: false,
                        });
                    });
                }
                RwOp::MemoryRead {
                    offset,
                    length,
                    signed,
                } => {
                    let addr = step.curr_nth_stack_value(0)?;
                    let mut value_le_bytes = vec![0; length as usize];
                    let mem_addr = offset as u64 + addr.as_u64();
                    step.curr_read_memory(mem_addr, value_le_bytes.as_mut_ptr(), length)?;
                    (0..length).for_each(|i| {
                        step.rw_rows.push(RwRow::Memory {
                            rw_counter,
                            is_write: false,
                            call_id,
                            memory_address: mem_addr + i as u64,
                            value: value_le_bytes[i as usize],
                            length,
                            signed,
                        });
                    });
                }
                RwOp::TableSizeRead(table_idx) => {
                    let table_size = step.read_table_size(table_idx);
                    step.rw_rows.push(RwRow::Table {
                        rw_counter,
                        is_write: false,
                        call_id,
                        address: (table_idx * 1024) as u64,
                        value: table_size as u64,
                    });
                }
                RwOp::TableSizeWrite(table_idx) => {
                    let table_size = step.read_table_size(table_idx);
                    let grow = step.curr_nth_stack_value(1)?;
                    step.rw_rows.push(RwRow::Table {
                        rw_counter,
                        is_write: true,
                        call_id,
                        address: (table_idx * 1024) as u64,
                        value: (table_size as u32 + grow.as_u32()) as u64,
                    });
                }
                RwOp::TableElemWrite(table_idx) => {
                    let elem_index = step.curr_nth_stack_value(1)?;
                    let value = step.curr_nth_stack_value(2)?;
                    step.rw_rows.push(RwRow::Table {
                        rw_counter,
                        is_write: true,
                        call_id,
                        address: (table_idx * 1024) as u64 + elem_index.as_u32() as u64 + 1,
                        value: value.as_u32() as u64,
                    });
                }
                RwOp::TableElemRead(table_idx) => {
                    let elem_index = step.curr_nth_stack_value(0)?;
                    let value = step.next_nth_stack_value(0)?;
                    step.rw_rows.push(RwRow::Table {
                        rw_counter,
                        is_write: false,
                        call_id,
                        address: (table_idx * 1024) as u64 + elem_index.as_u32() as u64 + 1,
                        value: value.as_u32() as u64,
                    });
                }
                _ => unreachable!("rw ops mapper is not implemented {:?}", rw_op),
            }
        }
        Ok(())
    }

    fn build_sys_call(
        &mut self,
        step: &mut ExecStep,
        sys_func: SysFuncIdx,
    ) -> Result<(), GadgetError> {
        match sys_func {
            SysFuncIdx::IMPORT_SYS_HALT => {
                self.build_rw_ops(step, vec![RwOp::StackRead(0)])?;
            }
            SysFuncIdx::IMPORT_SYS_WRITE => {}
            SysFuncIdx::IMPORT_SYS_READ => {
                // read 3 input params from the stack
                self.build_rw_ops(
                    step,
                    vec![RwOp::StackRead(0), RwOp::StackRead(0), RwOp::StackRead(0)],
                )?;
                let length = step.curr_nth_stack_value(0)?;
                let offset = step.curr_nth_stack_value(1)?;
                let target = step.curr_nth_stack_value(2)?;
                debug_assert_eq!(
                    step.next_trace.clone().unwrap().memory_changes[0].offset,
                    target.as_u32()
                );
                debug_assert_eq!(
                    step.next_trace.clone().unwrap().memory_changes[0].len,
                    length.as_u32()
                );
                let data = step.next_trace.clone().unwrap().memory_changes[0]
                    .data
                    .clone();
                let copy_rw_counter = step.next_rw_counter();
                // write result to the memory
                data.iter().enumerate().for_each(|(i, value)| {
                    step.rw_rows.push(RwRow::Memory {
                        rw_counter: step.next_rw_counter(),
                        is_write: true,
                        call_id: self.call_id,
                        memory_address: target.as_u64() + i as u64,
                        value: *value,
                        length: length.as_u32(),
                        signed: false,
                    });
                });
                // create copy row
                step.copy_rows.push(CopyRow {
                    tag: CopyTableTag::Input,
                    from_address: offset.as_u32(),
                    to_address: target.as_u32(),
                    length: length.as_u32(),
                    rw_counter: copy_rw_counter,
                    data,
                });
            }
            _ => {}
        }
        Ok(())
    }
}
