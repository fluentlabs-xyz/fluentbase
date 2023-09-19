use crate::impl_expr;
use fluentbase_rwasm::common::UntypedValue;
use std::{fmt, fmt::Formatter};
use strum_macros::EnumIter;
use std::mem::discriminant;

pub const N_RW_TABLE_TAG_BITS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, EnumIter)]
pub enum RwTableTag {
    Start = 1,
    Context,
    Memory,
    Stack,
    Global,
    Table,
}

impl_expr!(RwTableTag);

impl fmt::Display for RwTableTag {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            RwTableTag::Start => write!(f, "Start"),
            RwTableTag::Context => write!(f, "Context"),
            RwTableTag::Memory => write!(f, "Memory"),
            RwTableTag::Stack => write!(f, "Stack"),
            RwTableTag::Global => write!(f, "Global"),
            RwTableTag::Table => write!(f, "Table"),
        }
    }
}

impl Into<usize> for RwTableTag {
    fn into(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TagArg {
    Number(u32),
    // Using unsafe pointer and dynamic type to quickly solve problem.
    // TODO: this can be of course optimized and done in more smooth usual way.
    Query(*mut dyn std::any::Any),
}

impl Default for TagArg {
  fn default() -> Self {
    Self::Number(0)
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, EnumIter)]
#[repr(u64)]
pub enum RwTableContextTag {
    MemorySize = 1,
    ConsumedFuel,
    ProgramCounter,
    StackPointer,
    TableSize { table_index: TagArg },
}

impl_expr!(RwTableContextTag);

impl fmt::Display for RwTableContextTag {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            RwTableContextTag::MemorySize => write!(f, "MS"),
            RwTableContextTag::ConsumedFuel => write!(f, "CF"),
            RwTableContextTag::TableSize { .. } => write!(f, "TS"),
            RwTableContextTag::ProgramCounter => write!(f, "PC"),
            RwTableContextTag::StackPointer => write!(f, "SP"),
        }
    }
}

impl Into<usize> for RwTableContextTag {
    fn into(self) -> usize {
        Into::<u32>::into(self) as usize
    }
}

impl Into<u32> for RwTableContextTag {
    fn into(self) -> u32 {
        use RwTableContextTag::*;
        use TagArg::*;
        // TODO: use rust intetnals to do this.
        match self {
            MemorySize => 1,
            ConsumedFuel => 2,
            ProgramCounter => 3,
            StackPointer => 4,
            // TODO: solve this problem using rust internals.
            TableSize { table_index: Number(ti) } => (5 + ti).into(),
            // TODO: improve logic of translation with `dyn Any` where `Quert<F>` inside.
            TableSize { table_index: Query(_) } => 5,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum RwRow {
    /// Start
    Start { rw_counter: usize },
    /// Context
    Context {
        rw_counter: usize,
        is_write: bool,
        call_id: usize,
        tag: RwTableContextTag,
        value: u64,
    },
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
        signed: bool,
    },
    /// Table
    Table {
        rw_counter: usize,
        is_write: bool,
        call_id: usize,
        address: u64,
        value: u64,
        prev_value: u64, // Equal to zero in case of read operation.
    },
}

impl RwRow {
    pub fn value(&self) -> UntypedValue {
        match self {
            Self::Start { .. } => UntypedValue::default(),
            Self::Context { value, .. } => UntypedValue::from(*value),
            Self::Stack { value, .. } => *value,
            Self::Global { value, .. } => *value,
            Self::Memory { value: byte, .. } => UntypedValue::from(*byte),
            Self::Table { value, .. } => UntypedValue::from(*value),
            _ => unreachable!("{:?}", self),
        }
    }

    pub fn prev_value(&self) -> Option<UntypedValue> {
        match self {
            Self::Table { prev_value, .. } => Some(UntypedValue::from(*prev_value)),
            _ => None,
        }
    }

/*
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
*/

    pub fn rw_counter(&self) -> usize {
        match self {
            Self::Start { rw_counter }
            | Self::Context { rw_counter, .. }
            | Self::Memory { rw_counter, .. }
            | Self::Stack { rw_counter, .. }
            | Self::Global { rw_counter, .. } => *rw_counter,
            Self::Table { rw_counter, .. } => *rw_counter,
            _ => 0,
        }
    }

    pub fn is_write(&self) -> bool {
        match self {
            Self::Start { .. } => false,
            Self::Context { is_write, .. }
            | Self::Memory { is_write, .. }
            | Self::Stack { is_write, .. }
            | Self::Global { is_write, .. } => *is_write,
            Self::Table { is_write, .. } => *is_write,
            _ => false,
        }
    }

    pub fn tag(&self) -> RwTableTag {
        match self {
            Self::Start { .. } => RwTableTag::Start,
            Self::Context { .. } => RwTableTag::Context,
            Self::Memory { .. } => RwTableTag::Memory,
            Self::Stack { .. } => RwTableTag::Stack,
            Self::Global { .. } => RwTableTag::Global,
            Self::Table { .. } => RwTableTag::Table,
        }
    }

    pub fn id(&self) -> Option<usize> {
        match self {
            Self::Context { call_id, .. }
            | Self::Stack { call_id, .. }
            | Self::Global { call_id, .. }
            | Self::Table { call_id, .. }
            | Self::Memory { call_id, .. } => Some(*call_id),
            Self::Start { .. } => None,
        }
    }

    pub fn address(&self) -> Option<u32> {
        match self {
            Self::Context { tag, .. } => Some(Into::<u32>::into(*tag)),
            Self::Memory { memory_address, .. } => Some(*memory_address as u32),
            Self::Stack { stack_pointer, .. } => Some(*stack_pointer as u32),
            Self::Global { global_index, .. } => Some(*global_index as u32),
            Self::Table { address, .. } => Some(*address as u32),
            Self::Start { .. } => None,
        }
    }
}
