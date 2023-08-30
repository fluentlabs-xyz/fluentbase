use crate::{
    constraint_builder::AdviceColumn,
    state_circuit::{multiple_precision_integer::ToLimbs, param::N_LIMBS_RW_COUNTER},
    util::Field,
};
use halo2_proofs::{
    circuit::{Region, Value},
    plonk::Error,
};
use std::marker::PhantomData;

#[derive(Clone, Copy)]
pub struct MpiConfig<T, const N: usize>
where
    T: ToLimbs<N>,
{
    pub(crate) limbs: [AdviceColumn; N],
    pub(crate) _marker: PhantomData<T>,
}

impl MpiConfig<u32, N_LIMBS_RW_COUNTER> {
    pub fn assign<F: Field>(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        value: u32,
    ) -> Result<(), Error> {
        for (i, &limb) in value.to_limbs().iter().enumerate() {
            self.limbs[i].assign(region, offset, Value::known(F::from(limb as u64)));
        }
        Ok(())
    }

    /// Annotates columns of this gadget embedded within a circuit region.
    pub fn annotate_columns_in_region<F: Field>(&self, region: &mut Region<F>, prefix: &str) {
        let mut annotations = Vec::new();
        for (i, _) in self.limbs.iter().enumerate() {
            annotations.push(format!("MPI_limbs_u32_{}", i));
        }
        self.limbs
            .iter()
            .zip(annotations.iter())
            .for_each(|(col, ann)| region.name_column(|| format!("{}_{}", prefix, ann), *col));
    }
}
