/// This function will be removed in the future.
/// We keep it only for backward compatibility with testnet.
use crate::RuntimeContext;
use fluentbase_types::{keccak256, B256};
use rwasm::{StoreTr, TrapCode, Value};

pub fn syscall_hashing_keccak256_handler(
    caller: &mut impl StoreTr<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    let (data_offset, data_len, output_offset) = (
        params[0].i32().unwrap() as usize,
        params[1].i32().unwrap() as usize,
        params[2].i32().unwrap() as usize,
    );
    let data = caller.memory_read_into_vec(data_offset, data_len)?;
    let hash = syscall_hashing_keccak256_impl(&data);
    caller.memory_write(output_offset, hash.as_slice())?;
    Ok(())
}

pub fn syscall_hashing_keccak256_impl(data: &[u8]) -> B256 {
    keccak256(data)
}
