use crate::{
    gadgets::binary_number::BinaryNumberConfig,
    rw_builder::rw_row::{RwTableTag, N_RW_TABLE_TAG_BITS},
    state_circuit::{
        mpi_config::MpiConfig,
        param::{N_LIMBS_ADDRESS, N_LIMBS_ID, N_LIMBS_RW_COUNTER},
    },
    util::Field,
};

#[derive(Clone)]
pub struct SortKeysConfig<F: Field> {
    pub(crate) id: MpiConfig<F, u32, { N_LIMBS_ID }>,
    pub(crate) tag: BinaryNumberConfig<RwTableTag, { N_RW_TABLE_TAG_BITS }>,
    pub(crate) address: MpiConfig<F, u32, { N_LIMBS_ADDRESS }>,
    pub(crate) rw_counter: MpiConfig<F, u32, { N_LIMBS_RW_COUNTER }>,
}
