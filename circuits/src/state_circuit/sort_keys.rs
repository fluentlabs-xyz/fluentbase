use crate::{
    gadgets::binary_number::BinaryNumberConfig,
    state_circuit::{
        mpi_config::MpiConfig,
        param::{N_LIMBS_ADDRESS, N_LIMBS_ID, N_LIMBS_RW_COUNTER},
        tag::RwTableTag,
    },
    util::Field,
};

#[derive(Clone)]
pub struct SortKeysConfig<F: Field> {
    pub(crate) id: MpiConfig<F, u32, { N_LIMBS_ID }>,
    pub(crate) tag: BinaryNumberConfig<RwTableTag, 4>,
    pub(crate) address: MpiConfig<F, u32, { N_LIMBS_ADDRESS }>,
    pub(crate) rw_counter: MpiConfig<F, u32, { N_LIMBS_RW_COUNTER }>,
}
