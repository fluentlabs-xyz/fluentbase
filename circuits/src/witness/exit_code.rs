use crate::util::{unroll_to_hash_input, Field};
use itertools::Itertools;
use poseidon_circuit::HASHABLE_DOMAIN_SPEC;

#[derive(Clone, Default, Debug)]
pub struct UnrolledExitCode<F: Field> {
    exit_code: i32,
    hash: F,
    words: Vec<[F; 2]>,
}

impl<F: Field> UnrolledExitCode<F> {
    pub fn new(exit_code: i32) -> Self {
        let exit_code_bytes = exit_code.to_be_bytes().to_vec();
        let words = unroll_to_hash_input::<F, 31, 2>(exit_code_bytes.iter().copied());
        let hash = F::hash_msg(
            words.iter().flatten().copied().collect_vec().as_slice(),
            Some(exit_code_bytes.len() as u128 * HASHABLE_DOMAIN_SPEC),
        );
        Self {
            exit_code,
            hash,
            words,
        }
    }

    pub fn exit_code(&self) -> i32 {
        self.exit_code
    }

    pub fn hash(&self) -> F {
        self.hash
    }

    pub fn words(&self) -> &Vec<[F; 2]> {
        &self.words
    }
}
