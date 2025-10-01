use crate::RuntimeContext;
use fluentbase_sdk::B256;
use rwasm::{Store, TrapCode, Value};

pub struct SyscallBlake3;

impl SyscallBlake3 {
    pub fn fn_handler(
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
        let hash = Self::fn_impl(&data);
        caller.memory_write(output_offset, hash.as_slice())?;
        Ok(())
    }

    pub fn fn_impl(data: &[u8]) -> B256 {
        let mut hasher = blake3::Hasher::default();
        hasher.update(data);
        hasher.finalize().as_bytes().into()
    }
}
