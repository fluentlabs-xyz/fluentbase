use crate::util::{unroll_to_hash_input, Field};

#[derive(Clone, Default, Debug)]
pub struct UnrolledRawBytes<F: Field, const N: usize> {
    length: usize,
    hash: F,
    words: Vec<[F; 2]>,
}

impl<F: Field, const N: usize> UnrolledRawBytes<F, N> {
    pub fn new(input: &Vec<u8>) -> Self {
        let words = unroll_to_hash_input::<F, N, 2>(input.iter().copied());
        // let hash = F::hash_msg(
        //     words.iter().flatten().copied().collect_vec().as_slice(),
        //     Some(input.len() as u128 * HASHABLE_DOMAIN_SPEC),
        // );
        Self {
            length: input.len(),
            hash: F::zero(),
            words,
        }
    }

    pub fn length(&self) -> usize {
        self.length
    }

    pub fn hash(&self) -> F {
        self.hash
    }

    pub fn words(&self) -> &Vec<[F; 2]> {
        &self.words
    }
}
