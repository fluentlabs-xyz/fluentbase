use crate::{
    constraint_builder::{ConstraintBuilder, SelectorColumn},
    gadgets::poseidon::{PoseidonLookup, PoseidonTable},
    util::unroll_to_hash_input,
};
use halo2_proofs::{
    arithmetic::FieldExt,
    circuit::{Chip, Layouter},
    halo2curves::bn256::Fr,
    plonk::{ConstraintSystem, Error},
};
use hash_circuit::hash::{PoseidonHashChip, PoseidonHashConfig, PoseidonHashTable};

#[derive(Clone)]
pub struct RwasmRuntimeConfig {
    selector: SelectorColumn,
    poseidon_config: PoseidonHashConfig<Fr>,
    poseidon_table: PoseidonTable,
}

impl RwasmRuntimeConfig {
    pub fn configure(cs: &mut ConstraintSystem<Fr>) -> Self {
        let selector = SelectorColumn(cs.fixed_column());
        // let mut cb = ConstraintBuilder::new(selector);
        // cb.build(cs);
        let poseidon_table = PoseidonTable::configure(cs);
        let poseidon_config =
            PoseidonHashConfig::configure_sub(cs, poseidon_table.table_columns(), 32);
        Self {
            selector,
            poseidon_config,
            poseidon_table,
        }
    }

    pub fn assign(
        &self,
        layouter: &mut impl Layouter<Fr>,
        rwasm_binary: &[u8],
    ) -> Result<(), Error> {
        let hash_input = unroll_to_hash_input::<Fr, 31, 2>(rwasm_binary);
        let mut poseidon_hash_table = PoseidonHashTable::default();
        poseidon_hash_table.stream_inputs(hash_input, rwasm_binary.len() as u64, 31);
        let poseidon_hash_chip = PoseidonHashChip::construct(
            self.poseidon_config.clone(),
            &poseidon_hash_table,
            rwasm_binary.len(),
        );
        poseidon_hash_chip.load(layouter)?;

        layouter.assign_region(
            || "runtime circuit",
            |mut region| {
                // for offset in 1..rwasm_binary.len() {
                //     self.selector.enable(&mut region, offset);
                // }

                Ok(())
            },
        )
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn test() {}
}
