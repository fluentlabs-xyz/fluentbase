/// This function will be removed in the future.
/// We keep it only for backward compatibility with testnet.
use crate::RuntimeContext;
use fluentbase_types::B256;
use rwasm::{Store, TrapCode, Value};

pub fn syscall_hashing_blak3_handler(
    caller: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    let (data_offset, data_len, output_offset) = (
        params[0].i32().unwrap() as usize,
        params[1].i32().unwrap() as usize,
        params[2].i32().unwrap() as usize,
    );
    let mut data = vec![0u8; data_len];
    caller.memory_read(data_offset, &mut data)?;
    let hash = syscall_hashing_blake3_handler(&data);
    caller.memory_write(output_offset, hash.as_slice())?;
    Ok(())
}

pub fn syscall_hashing_blake3_handler(data: &[u8]) -> B256 {
    let mut hasher = blake3::Hasher::default();
    hasher.update(data);
    hasher.finalize().as_bytes().into()
}
