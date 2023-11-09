use crate::{instruction::exported_memory_vec, RuntimeContext};
use fluentbase_poseidon::{poseidon_hash, Hashable};
use fluentbase_rwasm::{common::Trap, Caller};
use halo2curves::{bn256::Fr, group::ff::PrimeField};
use keccak_hash::write_keccak;

pub(crate) fn crypto_keccak256(
    mut caller: Caller<'_, RuntimeContext>,
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

pub(crate) fn crypto_poseidon(
    mut caller: Caller<'_, RuntimeContext>,
    data_offset: i32,
    data_len: i32,
    output_offset: i32,
) -> Result<(), Trap> {
    let data = exported_memory_vec(&mut caller, data_offset as usize, data_len as usize);
    let hash = poseidon_hash(data.as_slice());
    caller.write_memory(output_offset as usize, hash.as_slice());
    Ok(())
}

pub(crate) fn crypto_poseidon2(
    mut caller: Caller<'_, RuntimeContext>,
    fa_offset: i32,
    fb_offset: i32,
    domain_offset: i32,
    output_offset: i32,
) -> Result<(), Trap> {
    let fa_data =
        TryInto::<[u8; 32]>::try_into(exported_memory_vec(&mut caller, fa_offset as usize, 32))
            .map_err(|_e| Trap::new(format!("failed to get fa_offset param")))?;
    let fb_data =
        TryInto::<[u8; 32]>::try_into(exported_memory_vec(&mut caller, fb_offset as usize, 32))
            .map_err(|_e| Trap::new(format!("failed to get fb_offset param")))?;
    let fdomain_data =
        TryInto::<[u8; 32]>::try_into(exported_memory_vec(&mut caller, domain_offset as usize, 32))
            .map_err(|_e| Trap::new(format!("failed to get fdomain_offset param")))?;

    let fa = Fr::from_bytes(&fa_data);
    let fa = if fa.is_some().into() {
        fa.unwrap()
    } else {
        return Err(Trap::new(format!("failed to get fa param")));
    };
    let fb = Fr::from_bytes(&fb_data);
    let fb = if fb.is_some().into() {
        fb.unwrap()
    } else {
        return Err(Trap::new(format!("failed to get fb param")));
    };
    let fdomain = Fr::from_bytes(&fdomain_data);
    let fdomain = if fdomain.is_some().into() {
        fdomain.unwrap()
    } else {
        return Err(Trap::new(format!("failed to get fdomain param")));
    };

    let hasher = Fr::hasher();
    let h2 = hasher.hash([fa, fb], fdomain);
    let hash = h2.to_repr();

    caller.write_memory(output_offset as usize, hash.as_slice());

    Ok(())
}

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
