use super::{BinaryQuery, Query};
use halo2_proofs::{
    arithmetic::FieldExt,
    circuit::{Region, Value},
    plonk::{Advice, Column, Fixed},
};
use std::fmt::Debug;

#[derive(Clone, Copy)]
pub struct SelectorColumn(pub Column<Fixed>);

impl SelectorColumn {
    pub fn current<F: FieldExt>(self) -> BinaryQuery<F> {
        self.rotation(0)
    }

    pub fn next<F: FieldExt>(self) -> BinaryQuery<F> {
        self.rotation(1)
    }

    pub fn rotation<F: FieldExt>(self, i: i32) -> BinaryQuery<F> {
        BinaryQuery(Query::Fixed(self.0, i))
    }

    pub fn enable<F: FieldExt>(&self, region: &mut Region<'_, F>, offset: usize) {
        region
            .assign_fixed(|| "selector", self.0, offset, || Value::known(F::one()))
            .expect("failed enable selector");
    }
}

#[derive(Clone, Copy)]
pub struct FixedColumn(pub Column<Fixed>);

impl FixedColumn {
    pub fn rotation<F: FieldExt>(self, i: i32) -> Query<F> {
        Query::Fixed(self.0, i)
    }

    pub fn current<F: FieldExt>(self) -> Query<F> {
        self.rotation(0)
    }

    pub fn previous<F: FieldExt>(self) -> Query<F> {
        self.rotation(-1)
    }

    pub fn assign<F: FieldExt, T: Copy + TryInto<F>>(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        value: T,
    ) where
        <T as TryInto<F>>::Error: Debug,
    {
        region
            .assign_fixed(
                || "asdfasdfawe",
                self.0,
                offset,
                || Value::known(value.try_into().unwrap()),
            )
            .expect("failed assign_fixed");
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AdviceColumn(pub Column<Advice>);

impl AdviceColumn {
    pub fn rotation<F: FieldExt>(self, i: i32) -> Query<F> {
        Query::Advice(self.0, i)
    }

    pub fn expr<F: FieldExt>(self) -> Query<F> {
        self.rotation(0)
    }

    pub fn current<F: FieldExt>(self) -> Query<F> {
        self.rotation(0)
    }

    pub fn previous<F: FieldExt>(self) -> Query<F> {
        self.rotation(-1)
    }

    pub fn next<F: FieldExt>(self) -> Query<F> {
        self.rotation(1)
    }

    pub fn delta<F: FieldExt>(self) -> Query<F> {
        self.current() - self.previous()
    }

    pub fn assign<F: FieldExt, T: Copy + TryInto<F>>(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        value: T,
    ) where
        <T as TryInto<F>>::Error: Debug,
    {
        region
            .assign_advice(
                || "advice",
                self.0,
                offset,
                || Value::known(value.try_into().unwrap()),
            )
            .expect("failed assign_advice");
    }
}

#[derive(Clone, Copy)]
pub struct AdviceColumnPhase2(pub Column<Advice>);

impl AdviceColumnPhase2 {
    fn rotation<F: FieldExt>(self, i: i32) -> Query<F> {
        Query::Advice(self.0, i)
    }

    pub fn current<F: FieldExt>(self) -> Query<F> {
        self.rotation(0)
    }

    pub fn previous<F: FieldExt>(self) -> Query<F> {
        self.rotation(-1)
    }

    pub fn assign<F: FieldExt>(&self, region: &mut Region<'_, F>, offset: usize, value: Value<F>) {
        region
            .assign_advice(|| "second phase advice", self.0, offset, || value)
            .expect("failed assign_advice");
    }
}
