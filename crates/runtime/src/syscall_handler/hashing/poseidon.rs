use crate::{syscall_handler::syscall_process_exit_code, RuntimeContext};
use fluentbase_types::{ExitCode, B256};
use rwasm::{Store, TrapCode, TypedCaller, Value};
use solana_poseidon::HASH_BYTES;

pub struct SyscallPoseidon;

impl SyscallPoseidon {
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let (parameters, endianness, data_offset, data_len, output_offset) = (
            params[0].i32().unwrap() as usize,
            params[1].i32().unwrap() as usize,
            params[2].i32().unwrap() as usize,
            params[3].i32().unwrap() as usize,
            params[4].i32().unwrap() as usize,
        );
        if data_len % HASH_BYTES != 0 {
            result[0] = Value::I32(1i32);
            syscall_process_exit_code(caller, ExitCode::MalformedBuiltinParams);
            return Ok(());
        }
        let mut data = vec![0; data_len];
        caller.memory_read(data_offset, &mut data)?;
        let hash = Self::fn_impl(parameters as u64, endianness as u64, &data);
        match hash {
            Ok(v) => {
                caller.memory_write(output_offset, v.as_slice())?;
            }
            Err(e) => {
                syscall_process_exit_code(caller, e);
            }
        };
        result[0] = Value::I32(hash.is_err() as i32);

        Ok(())
    }

    pub fn fn_impl(parameters: u64, endianness: u64, data: &[u8]) -> Result<B256, ExitCode> {
        let parameters: solana_poseidon::Parameters = parameters
            .try_into()
            .map_err(|_| ExitCode::MalformedBuiltinParams)?;
        let endianness: solana_poseidon::Endianness = endianness
            .try_into()
            .map_err(|_| ExitCode::MalformedBuiltinParams)?;

        if data.len() % HASH_BYTES != 0 {
            return Err(ExitCode::MalformedBuiltinParams);
        };
        let data = data.chunks(HASH_BYTES).map(|v| v).collect::<Vec<&[u8]>>();

        let hash = match solana_poseidon::hashv(parameters, endianness, &data) {
            Ok(hash) => hash,
            Err(_e) => {
                return Err(ExitCode::MalformedBuiltinParams);
            }
        };
        let mut hash_result = [0u8; 32];
        hash_result.copy_from_slice(&hash.to_bytes());
        Ok(hash_result.into())
    }
}
