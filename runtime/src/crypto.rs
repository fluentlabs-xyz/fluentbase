use crate::{instruction::exported_memory_vec, poseidon_hash::poseidon_hash, RuntimeContext};
use fluentbase_rwasm::{common::Trap, Caller};
use halo2curves::bn256::Fr;
use keccak_hash::write_keccak;
use poseidon::Poseidon;

pub(crate) fn crypto_keccak(
    mut caller: Caller<'_, RuntimeContext>,
    data_offset: i32,
    data_len: i32,
    output_offset: i32,
) -> Result<i32, Trap> {
    let data = exported_memory_vec(&mut caller, data_offset as usize, data_len as usize);
    let mut dest = [0u8; 32];
    write_keccak(data, &mut dest);
    caller.write_memory(output_offset as usize, dest.as_slice());

    Ok(dest.len() as i32)
}

pub(crate) fn crypto_poseidon(
    mut caller: Caller<'_, RuntimeContext>,
    data_offset: i32,
    data_len: i32,
    output_offset: i32,
) -> Result<i32, Trap> {
    let data = exported_memory_vec(&mut caller, data_offset as usize, data_len as usize);

    let hash = poseidon_hash(data.as_slice());

    caller.write_memory(output_offset as usize, hash.as_slice());

    Ok(hash.len() as i32)
}
