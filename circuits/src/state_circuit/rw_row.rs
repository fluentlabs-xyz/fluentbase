use crate::state_circuit::tag::RwTableTag;
use fluentbase_rwasm::common::UntypedValue;

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
        local_index: usize,
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
        byte: u8,
    },
}

impl RwRow {
    pub fn stack_value(&self) -> UntypedValue {
        match self {
            Self::Stack { value, .. } => *value,
            _ => unreachable!("{:?}", self),
        }
    }

    pub(crate) fn local_value(&self) -> (UntypedValue, usize) {
        match self {
            Self::Stack {
                value, local_index, ..
            } => (*value, *local_index),
            _ => unreachable!(),
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
            Self::Memory { byte, .. } => *byte,
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
