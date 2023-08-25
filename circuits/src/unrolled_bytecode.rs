use crate::{
    poseidon_circuit::{HASH_BLOCK_STEP_SIZE, HASH_BYTES_IN_FIELD},
    util::{unroll_to_hash_input, Field},
};
use fluentbase_rwasm::rwasm::{InstructionSet, ReducedModuleReader, ReducedModuleTrace};
use itertools::Itertools;
use poseidon_circuit::HASHABLE_DOMAIN_SPEC;
use std::{iter, marker::PhantomData};

#[derive(Clone, Default, Debug)]
pub struct UnrolledBytecode<F: Field> {
    original_bytecode: Vec<u8>,
    read_traces: Vec<ReducedModuleTrace>,
    instruction_set: InstructionSet,
    _pd: PhantomData<F>,
}

impl<F: Field> UnrolledBytecode<F> {
    pub fn new(bytecode: &[u8]) -> Self {
        let mut module_reader = ReducedModuleReader::new(bytecode);
        let mut traces: Vec<ReducedModuleTrace> = Vec::new();
        loop {
            let trace = match module_reader.trace_opcode() {
                Some(trace) => trace,
                None => break,
            };
            traces.push(trace);
        }
        Self {
            original_bytecode: bytecode.to_vec(),
            read_traces: traces,
            instruction_set: module_reader.instruction_set,
            _pd: Default::default(),
        }
    }

    pub fn len(&self) -> usize {
        self.original_bytecode.len()
    }

    pub fn read_traces(&self) -> &Vec<ReducedModuleTrace> {
        &self.read_traces
    }

    pub fn hash_traces(&self) -> Vec<[F; 2]> {
        unroll_to_hash_input::<F, { HASH_BYTES_IN_FIELD }, 2>(
            self.original_bytecode.iter().copied(),
        )
    }

    pub fn code_hash(&self) -> F {
        let items = self.hash_traces().iter().flatten().copied().collect_vec();
        F::hash_msg(
            items.as_slice(),
            Some(self.len() as u128 * HASHABLE_DOMAIN_SPEC),
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::util::unroll_to_hash_input;
    use halo2_proofs::halo2curves::bn256::Fr;
    use itertools::Itertools;
    use poseidon_circuit::{hash::MessageHashable, HASHABLE_DOMAIN_SPEC};

    #[test]
    fn test_code_hash() {
        let bytecode: [u8; 3] = [0x01, 0x02, 0x03];
        let unrolled = unroll_to_hash_input::<Fr, 31, 2>(bytecode.iter().copied());
        let items = unrolled.iter().flatten().copied().collect_vec();
        let hash = Fr::hash_msg(
            items.as_slice(),
            Some(bytecode.len() as u128 * HASHABLE_DOMAIN_SPEC),
        );
        assert_eq!(
            format!("{:?}", hash),
            "0x214a64c83da9032d8a65e53dd1b33ad80bc3f48f487598e8c054698afbbab2bd"
        );
    }
}
