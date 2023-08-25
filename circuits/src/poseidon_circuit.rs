use crate::{gadgets::poseidon::PoseidonTable, unrolled_bytecode::UnrolledBytecode, util::Field};
use halo2_proofs::{
    circuit::Layouter,
    plonk::{ConstraintSystem, Error},
};
use poseidon_circuit::hash::{PoseidonHashChip, PoseidonHashConfig, PoseidonHashTable};

pub const HASH_BYTES_IN_FIELD: usize = 9;
pub const HASH_BLOCK_STEP_SIZE: usize = 2 * HASH_BYTES_IN_FIELD;

#[derive(Clone)]
pub struct PoseidonCircuitConfig<F: Field> {
    poseidon_config: PoseidonHashConfig<F>,
    poseidon_table: PoseidonTable,
}

impl<F: Field> PoseidonCircuitConfig<F> {
    pub fn configure(cs: &mut ConstraintSystem<F>, poseidon_table: PoseidonTable) -> Self {
        let poseidon_config = PoseidonHashConfig::configure_sub(
            cs,
            poseidon_table.table_columns(),
            HASH_BLOCK_STEP_SIZE,
        );
        Self {
            poseidon_config,
            poseidon_table,
        }
    }

    pub fn assign_bytecode(
        &self,
        layouter: &mut impl Layouter<F>,
        bytecode: &UnrolledBytecode<F>,
    ) -> Result<(), Error> {
        let mut poseidon_hash_table = PoseidonHashTable::default();
        let hash_traces = bytecode.hash_traces();
        let code_hash = bytecode.code_hash();
        poseidon_hash_table.stream_inputs_with_check(
            &hash_traces,
            Some(code_hash),
            bytecode.len() as u64,
            HASH_BLOCK_STEP_SIZE,
        );

        let poseidon_hash_chip = PoseidonHashChip::<'_, F, { HASH_BYTES_IN_FIELD }>::construct(
            self.poseidon_config.clone(),
            &poseidon_hash_table,
            hash_traces.len() + 1,
        );
        poseidon_hash_chip.load(layouter)?;
        Ok(())
    }
}
