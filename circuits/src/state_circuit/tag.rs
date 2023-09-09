use crate::{
    constraint_builder::{Query, ToExpr},
    util::Field,
};
use std::{fmt, fmt::Formatter};
use strum_macros::EnumIter;

pub const N_RW_TABLE_TAG_BITS: usize = 4;

/// Tag to identify the operation type in a RwTable row
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, EnumIter)]
pub enum RwTableTag {
    Start = 1,
    Memory,
    Stack,
    Global,
    Table,
}

impl fmt::Display for RwTableTag {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            RwTableTag::Start => write!(f, "Start"),
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

impl ToExpr for RwTableTag {
    fn expr<F: Field>(&self) -> Query<F> {
        Query::Constant(F::from(*self as u64))
    }
}
