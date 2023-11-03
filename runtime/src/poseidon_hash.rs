use halo2curves::{bn256::Fr, group::ff::PrimeField};
use poseidon::Poseidon;

pub fn poseidon_hash(data: &[u8]) -> [u8; 32] {
    let mut hasher = Poseidon::<Fr, 3, 2>::new(8, 56);
    const CHUNK_LEN: usize = 32;
    for chunk in data.chunks(CHUNK_LEN).into_iter() {
        let chunk: [u8; CHUNK_LEN] = if chunk.len() == CHUNK_LEN {
            chunk.try_into().unwrap()
        } else {
            let mut tmp_chunk = [0u8; CHUNK_LEN];
            // be repr
            tmp_chunk[..chunk.len()].copy_from_slice(chunk);
            tmp_chunk
        };
        let v = Fr::from_bytes(&chunk).unwrap();
        hasher.update(&[v]);
    }
    let h = hasher.squeeze();
    h.to_bytes()
}

#[cfg(test)]
mod poseidon_tests {
    extern crate alloc;

    use crate::{poseidon_hash::poseidon_hash, poseidon_impl::hash::Hashable};
    use alloc::{vec, vec::Vec};
    use halo2curves::{bn256::Fr, group::ff::PrimeField};
    use poseidon::Poseidon;

    #[test]
    fn empty() {
        assert_eq!(
            poseidon_hash(&[0u8; 0]),
            [
                4, 44, 76, 53, 12, 109, 170, 99, 136, 141, 121, 133, 236, 148, 84, 202, 23, 196,
                176, 71, 252, 181, 29, 144, 148, 84, 57, 217, 35, 221, 200, 12
            ]
        );
    }

    #[test]
    fn with_content() {
        let data: Vec<u8> = From::from("hello world");
        let expected = vec![
            13, 147, 215, 180, 93, 24, 214, 147, 24, 205, 39, 124, 162, 132, 216, 125, 204, 48,
            249, 43, 252, 181, 68, 137, 189, 87, 214, 31, 48, 215, 193, 14,
        ];
        let hash = poseidon_hash(data.as_slice());

        assert_eq!(hash.as_slice(), expected.as_slice());
    }

    #[test]
    fn with_domain() {
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        let mut domain = [0u8; 32];

        a[0] = 1;
        b[0] = 1;
        domain[0] = 1;

        let fa = Fr::from_bytes(&a.try_into().unwrap()).unwrap();
        let fb = Fr::from_bytes(&b.try_into().unwrap()).unwrap();
        let fdomain = Fr::from_bytes(&domain.try_into().unwrap()).unwrap();

        let hasher = Fr::hasher();
        let h2 = hasher.hash([fa, fb], fdomain);
        let repr_h2 = h2.to_repr();

        let expected_repr = [
            160, 7, 117, 178, 129, 18, 242, 68, 19, 50, 96, 164, 159, 63, 81, 176, 201, 231, 26,
            133, 56, 207, 136, 8, 238, 33, 51, 5, 40, 31, 116, 6,
        ];
        assert_eq!(expected_repr, repr_h2);
    }
}
