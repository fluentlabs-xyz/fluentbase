use super::{BinaryQuery, ConstraintBuilder, Query};
use crate::util::Field;
use halo2_proofs::{
    circuit::{Region, Value},
    plonk::{Advice, Column, ConstraintSystem},
};

#[derive(Clone, Copy)]
pub struct BinaryColumn(pub Column<Advice>);

impl BinaryColumn {
    pub fn rotation<F: Field>(&self, i: i32) -> BinaryQuery<F> {
        BinaryQuery(Query::Advice(self.0, i))
    }

    pub fn current<F: Field>(&self) -> BinaryQuery<F> {
        self.rotation(0)
    }

    pub fn previous<F: Field>(&self) -> BinaryQuery<F> {
        self.rotation(-1)
    }

    pub fn next<F: Field>(&self) -> BinaryQuery<F> {
        self.rotation(1)
    }

    pub fn configure<F: Field>(
        cs: &mut ConstraintSystem<F>,
        _cb: &mut ConstraintBuilder<F>,
    ) -> Self {
        let advice_column = cs.advice_column();
        // TODO: constrain to be binary here...
        // cb.add_constraint()
        Self(advice_column)
    }

    pub fn assign<F: Field>(&self, region: &mut Region<'_, F>, offset: usize, value: bool) {
        region
            .assign_advice(|| "binary", self.0, offset, || Value::known(F::from(value)))
            .expect("failed assign_advice");
    }
}
