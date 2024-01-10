use crate::{instruction::exported_memory_vec, ExitCode, RuntimeContext};
use fluentbase_poseidon::{poseidon_hash, Hashable};
use fluentbase_rwasm::{common::Trap, Caller};
use halo2curves::{bn256::Fr, group::ff::PrimeField};
use k256::elliptic_curve::subtle::CtOption;
use keccak_hash::write_keccak;

pub(crate) fn crypto_keccak256<T>(
    mut caller: Caller<'_, RuntimeContext<T>>,
    data_offset: i32,
    data_len: i32,
    output_offset: i32,
) -> Result<(), Trap> {
    let data = exported_memory_vec(&mut caller, data_offset as usize, data_len as usize);
    let mut dest = [0u8; 32];
    write_keccak(data, &mut dest);
    caller.write_memory(output_offset as usize, dest.as_slice());
    Ok(())
}

pub(crate) fn crypto_poseidon<T>(
    mut caller: Caller<'_, RuntimeContext<T>>,
    data_offset: i32,
    data_len: i32,
    output_offset: i32,
) -> Result<(), Trap> {
    let data = exported_memory_vec(&mut caller, data_offset as usize, data_len as usize);
    let hash = poseidon_hash(data.as_slice());
    caller.write_memory(output_offset as usize, hash.as_slice());
    Ok(())
}

pub(crate) fn crypto_poseidon2<T>(
    mut caller: Caller<'_, RuntimeContext<T>>,
    fa_offset: i32,
    fb_offset: i32,
    fd_offset: i32,
    output_offset: i32,
) -> Result<(), Trap> {
    let mut fr_from_bytes = |offset| -> Result<Fr, Trap> {
        let fr_data: [u8; 32] = exported_memory_vec(&mut caller, offset as usize, 32)
            .try_into()
            .map_err(|_| <ExitCode as Into<Trap>>::into(ExitCode::PoseidonError))?;
        let fr = Fr::from_bytes(&fr_data);
        if fr.is_none().into() {
            return Err(<ExitCode as Into<Trap>>::into(ExitCode::PoseidonError));
        }
        Ok(fr.unwrap())
    };

    let fa = fr_from_bytes(fa_offset)?;
    let fb = fr_from_bytes(fb_offset)?;
    let fd = fr_from_bytes(fd_offset)?;

    let hasher = Fr::hasher();
    let h2 = hasher.hash([fa, fb], fd);
    let hash = h2.to_repr();

    caller.write_memory(output_offset as usize, hash.as_slice());
    Ok(())
}

#[cfg(test)]
mod keccak_tests {
    extern crate alloc;

    use alloc::{vec, vec::Vec};
    use fluentbase_poseidon::poseidon_hash;
    use keccak_hash::{keccak, write_keccak, H256, KECCAK_EMPTY};

    #[test]
    fn empty() {
        assert_eq!(keccak([0u8; 0]), KECCAK_EMPTY);
    }

    #[test]
    fn test_keccak256() {
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

    #[test]
    fn test_poseidon() {
        let data: Vec<u8> = From::from("hello world");
        let expected = vec![
            13, 147, 215, 180, 93, 24, 214, 147, 24, 205, 39, 124, 162, 132, 216, 125, 204, 48,
            249, 43, 252, 181, 68, 137, 189, 87, 214, 31, 48, 215, 193, 14,
        ];
        let hash = poseidon_hash(data.as_slice());
        assert_eq!(hash.as_slice(), expected.as_slice());
    }
}
