use crate::{
    constraint_builder::{Query, ToExpr},
    impl_expr,
    util::Field,
};
use fluentbase_rwasm::common::UntypedValue;
use std::{
    fmt,
    fmt::{Debug, Formatter},
};
use strum_macros::EnumIter;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, EnumIter)]
#[repr(u64)]
pub enum RwTableContextTag<Q: Default> {
    MemorySize = 1,
    ConsumedFuel,
    ProgramCounter,
    StackPointer,
    CallDepth,
    TableSize(Q),
}

impl<F: Field> ToExpr<F> for RwTableContextTag<Query<F>> {
    fn expr(&self) -> Query<F> {
        match self {
            RwTableContextTag::MemorySize => 1.expr(),
            RwTableContextTag::ConsumedFuel => 2.expr(),
            RwTableContextTag::ProgramCounter => 3.expr(),
            RwTableContextTag::StackPointer => 4.expr(),
            RwTableContextTag::CallDepth => 5.expr(),
            RwTableContextTag::TableSize(table_index) => {
                6.expr() + table_index.clone() * 256.expr()
            }
        }
    }
}

impl RwTableContextTag<u32> {
    pub(crate) fn address(&self) -> u32 {
        match self {
            RwTableContextTag::MemorySize => 1,
            RwTableContextTag::ConsumedFuel => 2,
            RwTableContextTag::ProgramCounter => 3,
            RwTableContextTag::StackPointer => 4,
            RwTableContextTag::CallDepth => 5,
            RwTableContextTag::TableSize(table_index) => 6 + table_index * 256,
        }
    }
}

impl<Q: Default + Debug> fmt::Display for RwTableContextTag<Q> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            RwTableContextTag::MemorySize => write!(f, "MS"),
            RwTableContextTag::ConsumedFuel => write!(f, "CF"),
            RwTableContextTag::TableSize(v) => write!(f, "TS({:?})", v),
            RwTableContextTag::ProgramCounter => write!(f, "PC"),
            RwTableContextTag::StackPointer => write!(f, "SP"),
            RwTableContextTag::CallDepth => write!(f, "CD"),
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
        call_id: u32,
        tag: RwTableContextTag<u32>,
        value: u64,
    },
    /// Stack
    Stack {
        rw_counter: usize,
        is_write: bool,
        call_id: u32,
        stack_pointer: usize,
        value: UntypedValue,
    },
    /// Global
    Global {
        rw_counter: usize,
        is_write: bool,
        call_id: u32,
        global_index: usize,
        value: UntypedValue,
    },
    /// Memory
    Memory {
        rw_counter: usize,
        is_write: bool,
        call_id: u32,
        memory_address: u64,
        value: u8,
        signed: bool,
    },
    /// Table
    Table {
        rw_counter: usize,
        is_write: bool,
        call_id: u32,
        address: u64,
        value: u64,
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

    pub fn id(&self) -> Option<u32> {
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
            Self::Context { tag, .. } => Some(tag.address()),
            Self::Memory { memory_address, .. } => Some(*memory_address as u32),
            Self::Stack { stack_pointer, .. } => Some(*stack_pointer as u32),
            Self::Global { global_index, .. } => Some(*global_index as u32),
            Self::Table { address, .. } => Some(*address as u32),
            Self::Start { .. } => None,
        }
    }
}
