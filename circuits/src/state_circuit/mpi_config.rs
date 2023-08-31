use crate::{
    constraint_builder::{AdviceColumn, ConstraintBuilder, Query},
    gadgets::range_check::RangeCheckLookup,
    state_circuit::param::N_LIMBS_RW_COUNTER,
    util::Field,
};
use halo2_proofs::{
    circuit::Region,
    plonk::{ConstraintSystem, Error},
};
use itertools::Itertools;
use std::marker::PhantomData;

pub trait ToLimbs<const N: usize> {
    fn to_limbs(&self) -> [u16; N];
}

impl ToLimbs<N_LIMBS_RW_COUNTER> for u32 {
    fn to_limbs(&self) -> [u16; 2] {
        le_bytes_to_limbs(&self.to_le_bytes()).try_into().unwrap()
    }
}

#[derive(Clone, Copy)]
pub struct MpiConfig<F: Field, T, const N: usize>
where
    T: ToLimbs<N>,
{
    pub(crate) limbs: [AdviceColumn; N],
    pub(crate) _marker: PhantomData<T>,
    pub(crate) _marker2: PhantomData<F>,
}

impl<F: Field, T, const N: usize> MpiConfig<F, T, N>
where
    T: ToLimbs<N>,
{
    pub fn configure(
        cs: &mut ConstraintSystem<F>,
        cb: &mut ConstraintBuilder<F>,
        value: AdviceColumn,
        range_check_lookup: &impl RangeCheckLookup<F>,
    ) -> Self {
        let limbs = cb.advice_columns(cs);
        for limb in limbs.iter() {
            // cb.add_lookup(
            //     "mpi limb fits into u16",
            //     [limb.current()],
            //     range_check_lookup.lookup_u16_table(),
            // );
        }
        let q_limbs = limbs.map(|limb| limb.current());
        cb.assert_zero(
            "mpi value matches claimed limbs",
            value.current() - value_from_limbs(&q_limbs),
        );
        Self {
            limbs,
            _marker: PhantomData,
            _marker2: PhantomData,
        }
    }
}

impl<F: Field> MpiConfig<F, u32, { N_LIMBS_RW_COUNTER }> {
    pub fn assign(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        value: u32,
    ) -> Result<(), Error> {
        for (i, &limb) in value.to_limbs().iter().enumerate() {
            self.limbs[i].assign(region, offset, F::from(limb as u64));
        }
        Ok(())
    }
}

pub fn le_bytes_to_limbs(bytes: &[u8]) -> Vec<u16> {
    bytes
        .iter()
        .tuples()
        .map(|(lo, hi)| u16::from_le_bytes([*lo, *hi]))
        .collect()
}

pub fn value_from_limbs<F: Field>(limbs: &[Query<F>]) -> Query<F> {
    limbs.iter().rev().fold(Query::zero(), |result, limb| {
        limb.clone() + result * Query::from(1u64 << 16)
    })
}
