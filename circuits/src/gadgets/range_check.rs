use crate::{constraint_builder::Query, util::Field};

pub trait RangeCheckLookup<F: Field> {
    fn lookup_u8_table(&self) -> [Query<F>; 1];

    fn lookup_u16_table(&self) -> [Query<F>; 1];
}
