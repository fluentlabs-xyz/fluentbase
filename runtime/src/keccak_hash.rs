#[cfg(test)]
mod keccak_tests {
    extern crate alloc;

    use alloc::{vec, vec::Vec};
    use keccak_hash::{keccak, write_keccak, H256, KECCAK_EMPTY};

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
