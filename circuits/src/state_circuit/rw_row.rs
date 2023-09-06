use crate::{
    state_circuit::tag::RwTableTag,
    trace_step::{GadgetError, TraceStep},
};
use fluentbase_runtime::SysFuncIdx;
use fluentbase_rwasm::{common::UntypedValue, engine::bytecode::Instruction, RwOp};

#[derive(Clone, Copy, Debug)]
pub enum RwRow {
    /// Start
    Start { rw_counter: usize },
    /// Stack
    Stack {
        rw_counter: usize,
        is_write: bool,
        call_id: usize,
        stack_pointer: usize,
        value: UntypedValue,
    },
    /// Global
    Global {
        rw_counter: usize,
        is_write: bool,
        call_id: usize,
        global_index: usize,
        value: UntypedValue,
    },
    /// Memory
    Memory {
        rw_counter: usize,
        is_write: bool,
        call_id: usize,
        memory_address: u64,
        value: u8,
        length: u32,
        signed: bool,
    },
}

pub fn rw_rows_from_trace(
    res: &mut Vec<RwRow>,
    trace: &TraceStep,
    call_id: usize,
) -> Result<(), GadgetError> {
    let mut rw_ops = trace.instr().get_rw_ops();
    match trace.instr() {
        Instruction::Call(fn_idx) => {
            let sys_func = SysFuncIdx::from(*fn_idx);
            rw_ops.extend(sys_func.get_rw_rows());
        }
        _ => {}
    }
    let mut stack_reads = 0;
    let mut stack_writes = 0;
    for rw_op in rw_ops {
        match rw_op {
            RwOp::StackWrite(local_depth) => {
                let addr = trace.next_nth_stack_addr(stack_writes + local_depth as usize)?;
                let value = trace.next_nth_stack_value(stack_writes + local_depth as usize)?;
                res.push(RwRow::Stack {
                    rw_counter: res.len(),
                    is_write: true,
                    call_id,
                    stack_pointer: addr as usize,
                    value,
                });
                stack_writes += 1
            }
            RwOp::StackRead(local_depth) => {
                let addr = trace.curr_nth_stack_addr(stack_reads + local_depth as usize)?;
                let value = trace.curr_nth_stack_value(stack_reads + local_depth as usize)?;
                res.push(RwRow::Stack {
                    rw_counter: res.len(),
                    is_write: false,
                    call_id,
                    stack_pointer: addr as usize,
                    value,
                });
                stack_reads += 1;
            }
            RwOp::GlobalWrite(global_index) => {
                let value = trace.curr_nth_stack_value(0)?;
                res.push(RwRow::Global {
                    rw_counter: res.len(),
                    is_write: true,
                    call_id,
                    global_index: global_index as usize,
                    value,
                });
            }
            RwOp::GlobalRead(global_index) => {
                let value = trace.next_nth_stack_value(0)?;
                res.push(RwRow::Global {
                    rw_counter: res.len(),
                    is_write: false,
                    call_id,
                    global_index: global_index as usize,
                    value,
                });
            }
            RwOp::MemoryWrite { offset, length, .. } => {
                let value = trace.curr_nth_stack_value(0)?;
                let addr = trace.curr_nth_stack_value(1)?;
                let value_le_bytes = value.to_bits().to_le_bytes();
                (0..length).for_each(|i| {
                    res.push(RwRow::Memory {
                        rw_counter: res.len(),
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
                let value = trace.curr_nth_stack_value(0)?;
                let addr = trace.curr_nth_stack_value(1)?;
                let value_le_bytes = value.to_bits().to_le_bytes();
                (0..length).for_each(|i| {
                    res.push(RwRow::Memory {
                        rw_counter: res.len(),
                        is_write: false,
                        call_id,
                        memory_address: addr.as_u64() + offset as u64 + i as u64,
                        value: value_le_bytes[i as usize],
                        length,
                        signed,
                    });
                });
            }
            // RwOp::TableWrite => {}
            // RwOp::TableRead => {}
            _ => unreachable!("rw ops mapper is not implemented {:?}", rw_op),
        }
    }
    Ok(())
}

impl RwRow {
    pub fn value(&self) -> UntypedValue {
        match self {
            Self::Stack { value, .. } => *value,
            Self::Global { value, .. } => *value,
            Self::Memory { value: byte, .. } => UntypedValue::from(*byte),
            _ => unreachable!("{:?}", self),
        }
    }

    pub fn stack_value(&self) -> UntypedValue {
        match self {
            Self::Stack { value, .. } => *value,
            _ => unreachable!("{:?}", self),
        }
    }

    pub(crate) fn global_value(&self) -> (UntypedValue, usize) {
        match self {
            Self::Global {
                value,
                global_index,
                ..
            } => (*value, *global_index),
            _ => unreachable!(),
        }
    }

    pub fn memory_value(&self) -> u8 {
        match self {
            Self::Memory { value: byte, .. } => *byte,
            _ => unreachable!("{:?}", self),
        }
    }

    pub fn rw_counter(&self) -> usize {
        match self {
            Self::Start { rw_counter }
            | Self::Memory { rw_counter, .. }
            | Self::Stack { rw_counter, .. }
            | Self::Global { rw_counter, .. } => *rw_counter,
            _ => 0,
        }
    }

    pub fn is_write(&self) -> bool {
        match self {
            Self::Start { .. } => false,
            Self::Memory { is_write, .. }
            | Self::Stack { is_write, .. }
            | Self::Global { is_write, .. } => *is_write,
            _ => false,
        }
    }

    pub fn tag(&self) -> RwTableTag {
        match self {
            Self::Start { .. } => RwTableTag::Start,
            Self::Memory { .. } => RwTableTag::Memory,
            Self::Stack { .. } => RwTableTag::Stack,
            Self::Global { .. } => RwTableTag::Global,
        }
    }

    pub fn id(&self) -> Option<usize> {
        match self {
            Self::Stack { call_id, .. }
            | Self::Global { call_id, .. }
            | Self::Memory { call_id, .. } => Some(*call_id),
            Self::Start { .. } => None,
        }
    }

    pub fn address(&self) -> Option<u32> {
        match self {
            Self::Memory { memory_address, .. } => Some(*memory_address as u32),
            Self::Stack { stack_pointer, .. } => Some(*stack_pointer as u32),
            Self::Global { global_index, .. } => Some(*global_index as u32),
            Self::Start { .. } => None,
        }
    }
}
