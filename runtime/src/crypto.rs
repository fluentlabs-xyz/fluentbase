use crate::{
    instruction::exported_memory_vec,
    poseidon_hash::poseidon_hash,
    poseidon_impl::hash::Hashable,
    secp256k1_verify,
    RuntimeContext,
};
use fluentbase_rwasm::{common::Trap, Caller};
use halo2curves::{bn256::Fr, group::ff::PrimeField};
use keccak_hash::write_keccak;

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

pub(crate) fn crypto_poseidon_with_domain(
    mut caller: Caller<'_, RuntimeContext>,
    fa_offset: i32,
    fb_offset: i32,
    fdomain_offset: i32,
    output_offset: i32,
) -> Result<i32, Trap> {
    let fa_data =
        TryInto::<[u8; 32]>::try_into(exported_memory_vec(&mut caller, fa_offset as usize, 32))
            .map_err(|e| Trap::new(format!("failed to get fa_offset param")))?;
    let fb_data =
        TryInto::<[u8; 32]>::try_into(exported_memory_vec(&mut caller, fb_offset as usize, 32))
            .map_err(|e| Trap::new(format!("failed to get fb_offset param")))?;
    let fdomain_data = TryInto::<[u8; 32]>::try_into(exported_memory_vec(
        &mut caller,
        fdomain_offset as usize,
        32,
    ))
    .map_err(|e| Trap::new(format!("failed to get fdomain_offset param")))?;

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

    Ok(hash.len() as i32)
}

pub(crate) fn crypto_secp256k1_verify(
    mut caller: Caller<'_, RuntimeContext>,
    digest: i32,
    digest_len: i32,
    sig: i32,
    sig_len: i32,
    recid: i32,
    pk_expected: i32,
    pk_expected_len: i32,
) -> Result<i32, Trap> {
    let digest_data = exported_memory_vec(&mut caller, digest as usize, digest_len as usize);
    let sig_data = exported_memory_vec(&mut caller, sig as usize, sig_len as usize);
    let pk_expected_data =
        exported_memory_vec(&mut caller, pk_expected as usize, pk_expected_len as usize);

    let is_ok = secp256k1_verify(&digest_data, &sig_data, recid as u8, &pk_expected_data);

    Ok(is_ok as i32)
}
