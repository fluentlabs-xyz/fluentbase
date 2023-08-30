use crate::{
    constraint_builder::{AdviceColumn, FixedColumn, Query},
    gadgets::range_check::RangeCheckLookup,
    state_circuit::{mpi_config::MpiConfig, param::N_LIMBS_RW_COUNTER},
    util::Field,
};
use halo2_proofs::{
    circuit::Layouter,
    plonk::{ConstraintSystem, Error},
    poly::Rotation,
};
use itertools::Itertools;
use std::marker::PhantomData;

pub trait ToLimbs<const N: usize> {
    fn to_limbs(&self) -> [u16; N];
}

impl ToLimbs<{ N_LIMBS_RW_COUNTER }> for u32 {
    fn to_limbs(&self) -> [u16; 2] {
        le_bytes_to_limbs(&self.to_le_bytes()).try_into().unwrap()
    }
}

#[derive(Clone)]
pub struct Queries<F: Field, const N: usize> {
    pub limbs: [Query<F>; N],
    pub limbs_prev: [Query<F>; N],
}

impl<F: Field, const N: usize> Queries<F, N> {
    pub fn new<T: ToLimbs<N>>(config: MpiConfig<T, N>) -> Self {
        Self {
            limbs: config.limbs.map(|limb| limb.current()),
            limbs_prev: config.limbs.map(|limb| limb.previous()),
        }
    }
}

pub struct Chip<F: Field, T, const N: usize>
where
    T: ToLimbs<N>,
{
    config: MpiConfig<T, N>,
    _marker: PhantomData<F>,
}

impl<F: Field, T, const N: usize> Chip<F, T, N>
where
    T: ToLimbs<N>,
{
    pub fn construct(config: MpiConfig<T, N>) -> Self {
        Self {
            config,
            _marker: PhantomData,
        }
    }

    pub fn configure(
        meta: &mut ConstraintSystem<F>,
        selector: FixedColumn,
        value: AdviceColumn,
        lookup: &impl RangeCheckLookup<F>,
    ) -> MpiConfig<T, N> {
        let limbs = [0; N].map(|_| AdviceColumn(meta.advice_column()));

        for &limb in &limbs {
            lookup.range_check_u16(meta, "mpi limb fits into u16", |meta| {
                meta.query_advice(limb, Rotation::cur())
            });
        }
        meta.create_gate("mpi value matches claimed limbs", |meta| {
            let limbs = limbs.map(|limb| limb.current());
            vec![selector.current() * (value.current() - value_from_limbs(&limbs))]
        });

        MpiConfig {
            limbs,
            _marker: PhantomData,
        }
    }

    pub fn load(&self, _layouter: &mut impl Layouter<F>) -> Result<(), Error> {
        Ok(())
    }
}

fn le_bytes_to_limbs(bytes: &[u8]) -> Vec<u16> {
    bytes
        .iter()
        .tuples()
        .map(|(lo, hi)| u16::from_le_bytes([*lo, *hi]))
        .collect()
}

fn value_from_limbs<F: Field>(limbs: &[Query<F>]) -> Query<F> {
    limbs.iter().rev().fold(Query::zero(), |result, limb| {
        limb.clone() + result * Query::from(1u64 << 16)
    })
}
