use crate::util::Field;
use halo2_proofs::{
    circuit::{Layouter, Value},
    plonk::{Column, ConstraintSystem, Error, Expression, Fixed, VirtualCells},
    poly::Rotation,
};
use std::marker::PhantomData;

#[derive(Clone, Copy, Debug)]
pub struct Config {
    u16: Column<Fixed>,
}

impl Config {
    pub fn range_check_u16<F: Field>(
        &self,
        meta: &mut ConstraintSystem<F>,
        msg: &'static str,
        exp_fn: impl FnOnce(&mut VirtualCells<'_, F>) -> Expression<F>,
    ) {
        meta.lookup_any(msg, |meta| {
            let exp = exp_fn(meta);
            vec![(exp, meta.query_fixed(self.u16, Rotation::cur()))]
        });
    }
}

#[derive(Clone)]
pub struct Queries<F> {
    pub u16: Expression<F>,
}

impl<F: Field> Queries<F> {
    pub fn new(meta: &mut VirtualCells<'_, F>, c: Config) -> Self {
        Self {
            u16: meta.query_fixed(c.u16, Rotation::cur()),
        }
    }
}

pub struct Chip<F: Field> {
    config: Config,
    _marker: PhantomData<F>,
}

impl<F: Field> Chip<F> {
    pub fn construct(config: Config) -> Self {
        Self {
            config,
            _marker: PhantomData,
        }
    }

    pub fn configure(meta: &mut ConstraintSystem<F>) -> Config {
        let config = Config {
            u16: meta.fixed_column(),
        };
        meta.annotate_lookup_any_column(config.u16, || "LOOKUP_u16");
        config
    }

    pub fn load(&self, layouter: &mut impl Layouter<F>) -> Result<(), Error> {
        for (column, exponent) in [(self.config.u16, 16)] {
            layouter.assign_region(
                || format!("assign u{} fixed column", exponent),
                |mut region| {
                    for i in 0..(1 << exponent) {
                        region.assign_fixed(
                            || format!("assign {} in u{} fixed column", i, exponent),
                            column,
                            i,
                            || Value::known(F::from(i as u64)),
                        )?;
                    }
                    Ok(())
                },
            )?;
        }
        Ok(())
    }
}
