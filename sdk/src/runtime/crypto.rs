use crate::{CryptoPlatformSDK, SDK};
use halo2curves::{bn256::Fr, group::ff::PrimeField};
use keccak_hash::write_keccak;

use fluentbase_poseidon::*;

impl CryptoPlatformSDK for SDK {
    fn crypto_keccak256(data: &[u8], output: &mut [u8]) {
        write_keccak(data, output);
    }

    fn crypto_poseidon(data: &[u8], output: &mut [u8]) {
        let hash = poseidon_hash(data);
        output.copy_from_slice(&hash);
    }

    fn crypto_poseidon2(
        fa_data: &[u8; 32],
        fb_data: &[u8; 32],
        fdomain_data: &[u8; 32],
        output: &mut [u8],
    ) {
        let fa = Fr::from_bytes(fa_data);
        let fa = fa.unwrap();
        // let fa = if fa.is_some().into() {
        //     fa.unwrap()
        // } else {
        // };

        let fb = Fr::from_bytes(&fb_data);
        let fb = fb.unwrap();
        //     fb.unwrap()
        // } else {
        //     return Err(Trap::new(format!("failed to get fb param")));
        // };

        let fdomain = Fr::from_bytes(&fdomain_data);
        let fdomain = fdomain.unwrap();
        //     fdomain.unwrap()
        // } else {
        //     return Err(Trap::new(format!("failed to get fdomain param")));
        // };

        let hasher = Fr::hasher();
        let h2 = hasher.hash([fa, fb], fdomain);
        let hash = h2.to_repr();

        output.copy_from_slice(&hash);
    }
}

#[cfg(test)]
mod keccak_tests {
    extern crate alloc;

    use alloc::{vec, vec::Vec};
    use keccak_hash::{keccak, write_keccak, KECCAK_EMPTY};

    #[test]
    fn empty() {
        assert_eq!(keccak([0u8; 0]), KECCAK_EMPTY);
    }

    #[test]
    fn with_content() {
        let data: Vec<u8> = From::from("hello world");
        let expected = vec![
            0x47, 0x17, 0x32, 0x85, 0xa8, 0xd7, 0x34, 0x1e, 0x5e, 0x97, 0x2f, 0xc6, 0x77, 0x28,
            0x63, 0x84, 0xf8, 0x02, 0xf8, 0xef, 0x42, 0xa5, 0xec, 0x5f, 0x03, 0xbb, 0xfa, 0x25,
            0x4c, 0xb0, 0x1f, 0xad,
        ];
        let mut dest = [0u8; 32];
        write_keccak(data, &mut dest);

        assert_eq!(dest, expected.as_ref());
    }
}
