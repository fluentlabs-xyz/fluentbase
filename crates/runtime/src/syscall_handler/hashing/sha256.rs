use crate::RuntimeContext;
use fluentbase_types::B256;
use rwasm::{Store, TrapCode, TypedCaller, Value};
use sha2::{Digest, Sha256};

pub struct SyscallSha256;

impl SyscallSha256 {
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
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
        let hash = Self::fn_impl(&data);
        caller.memory_write(output_offset, hash.as_slice())?;
        Ok(())
    }

    pub fn fn_impl(data: &[u8]) -> B256 {
        let mut hasher = Sha256::default();
        hasher.update(data);
        let hash: [u8; 32] = hasher.finalize().into();
        hash.into()
    }
}
