use crate::{
    constraint_builder::AdviceColumn,
    gadgets::binary_number::BinaryNumberConfig,
    state_circuit::{
        multiple_precision_integer::MpiConfig,
        param::{N_LIMBS_ADDRESS, N_LIMBS_ID, N_LIMBS_RW_COUNTER},
        tag::RwTableTag,
    },
    util::Field,
};
use halo2_proofs::circuit::Region;

#[derive(Clone)]
pub struct SortKeysConfig {
    // pub(crate) tag: BinaryNumberConfig<RwTableTag, 4>,
    // pub(crate) id: MpiConfig<u32, { N_LIMBS_ID }>,
    // pub(crate) address: MpiConfig<u32, { N_LIMBS_ADDRESS }>,
    // pub(crate) field_tag: AdviceColumn,
    // pub(crate) rw_counter: MpiConfig<u32, { N_LIMBS_RW_COUNTER }>,
}

impl SortKeysConfig {
    /// Annotates this config within a circuit region.
    pub fn annotate_columns_in_region<F: Field>(&self, region: &mut Region<'_, F>, prefix: &str) {
        self.tag.annotate_columns_in_region(region, prefix);
        self.address.annotate_columns_in_region(region, prefix);
        self.id.annotate_columns_in_region(region, prefix);
        self.storage_key.annotate_columns_in_region(region, prefix);
        self.rw_counter.annotate_columns_in_region(region, prefix);
        region.name_column(|| format!("{}_field_tag", prefix), self.field_tag);
    }
}
