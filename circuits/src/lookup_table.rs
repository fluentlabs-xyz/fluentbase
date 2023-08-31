use crate::{constraint_builder::Query, util::Field};

pub const N_RWASM_LOOKUP_TABLE: usize = 4;

pub trait RwasmLookup<F: Field> {
    fn lookup_rwasm_table(&self) -> [Query<F>; N_RWASM_LOOKUP_TABLE];
}

pub const N_RW_LOOKUP_TABLE: usize = 7;

pub trait RwLookup<F: Field> {
    fn lookup_rw_table(&self) -> [Query<F>; N_RW_LOOKUP_TABLE];
}

pub enum LookupTable<F: Field> {
    Rwasm([Query<F>; N_RWASM_LOOKUP_TABLE]),
    Rw([Query<F>; N_RW_LOOKUP_TABLE]),
}
