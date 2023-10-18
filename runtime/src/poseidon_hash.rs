use halo2curves::bn256::Fr;
use poseidon::Poseidon;

pub fn poseidon_hash(data: &[u8]) -> [u8; 32] {
    let mut hasher = Poseidon::<Fr, 3, 2>::new(8, 56);
    const CHUNK_LEN: usize = 32;
    for chunk in data.chunks(CHUNK_LEN).into_iter() {
        println!("chunk in {:?}", chunk);
        let chunk: [u8; CHUNK_LEN] = if chunk.len() == CHUNK_LEN {
            chunk.try_into().unwrap()
        } else {
            let mut tmp_chunk = [0u8; CHUNK_LEN];
            // be repr
            tmp_chunk[..chunk.len()].copy_from_slice(chunk);
            tmp_chunk
        };
        println!("chunk out {:?} len {}", chunk, chunk.len());
        let v = Fr::from_bytes(&chunk).unwrap();
        hasher.update(&[v]);
    }
    let h = hasher.squeeze();
    h.to_bytes()
}

#[cfg(test)]
mod poseidon_tests {
    extern crate alloc;

    use crate::poseidon_hash::poseidon_hash;
    use alloc::{vec, vec::Vec};

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
}
