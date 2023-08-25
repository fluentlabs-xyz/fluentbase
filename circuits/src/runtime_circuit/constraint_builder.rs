use crate::{
    constraint_builder::{AdviceColumn, BinaryQuery, ConstraintBuilder, Query},
    util::Field,
};

pub struct OpConstraintBuilder<F: Field> {
    base: ConstraintBuilder<F>,
}

impl<F: Field> OpConstraintBuilder<F> {
    pub fn query_cell(&mut self) -> AdviceColumn {
        unreachable!("not implemented yet")
    }

    pub fn query_cell_phase2(&mut self) -> AdviceColumn {
        unreachable!("not implemented yet")
    }

    pub fn stack_push(&mut self, value: Query<F>) {
        unreachable!("not implemented yet")
    }
    pub fn stack_pop(&mut self, value: Query<F>) {
        unreachable!("not implemented yet")
    }
    pub fn stack_lookup(&mut self, is_write: Query<F>, address: Query<F>, value: Query<F>) {
        unreachable!("not implemented yet")
    }

    pub fn stack_pointer_offset(&self) -> Query<F> {
        unreachable!("not implemented yet")
    }

    pub fn require_equal(&mut self, name: &'static str, left: Query<F>, right: Query<F>) {
        self.base.assert_zero(name, left - right)
    }

    pub fn condition(&mut self, condition: Query<F>, configure: impl FnOnce(&mut Self)) {
        self.base.enter_condition(BinaryQuery(condition));
        configure(self);
        self.base.leave_condition();
    }
}

pub trait ToExpr {
    fn expr<F: Field>(&self) -> Query<F>;
}

macro_rules! impl_expr {
    ($ty:ty) => {
        impl ToExpr for $ty {
            fn expr<F: Field>(&self) -> Query<F> {
                Query::from(*self as u64)
            }
        }
    };
}

impl_expr!(u64);
impl_expr!(i64);
impl_expr!(u32);
impl_expr!(i32);
impl_expr!(u16);
impl_expr!(i16);
impl_expr!(u8);
impl_expr!(i8);
impl_expr!(usize);
impl_expr!(isize);
