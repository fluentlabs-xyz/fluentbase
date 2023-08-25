use crate::{
    constraint_builder::{
        AdviceColumn,
        AdviceColumnPhase2,
        BinaryQuery,
        ConstraintBuilder,
        FixedColumn,
        Query,
    },
    util::Field,
};
use halo2_proofs::plonk::ConstraintSystem;

pub struct OpConstraintBuilder<'cs, F: Field> {
    base: ConstraintBuilder<F>,
    cs: &'cs mut ConstraintSystem<F>,
}

impl<'cs, F: Field> OpConstraintBuilder<'cs, F> {
    pub fn query_cell(&mut self) -> AdviceColumn {
        self.base.advice_column(self.cs)
    }

    pub fn query_fixed(&mut self) -> FixedColumn {
        self.base.fixed_column(self.cs)
    }

    pub fn query_cell_phase2(&mut self) -> AdviceColumnPhase2 {
        self.base.advice_column_phase2(self.cs)
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

impl ToExpr for AdviceColumn {
    fn expr<F: Field>(&self) -> Query<F> {
        self.current()
    }
}
impl ToExpr for AdviceColumnPhase2 {
    fn expr<F: Field>(&self) -> Query<F> {
        self.current()
    }
}
impl ToExpr for FixedColumn {
    fn expr<F: Field>(&self) -> Query<F> {
        self.current()
    }
}
