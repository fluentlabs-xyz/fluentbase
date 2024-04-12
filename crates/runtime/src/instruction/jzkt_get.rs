use crate::RuntimeContext;
use fluentbase_types::IJournaledTrie;
use rwasm::{core::Trap, Caller};

pub struct JzktGet;

impl JzktGet {
    pub fn fn_handler<DB: IJournaledTrie>(
        mut caller: Caller<'_, RuntimeContext<DB>>,
        key32_offset: u32,
        field: u32,
        output32_offset: u32,
    ) -> Result<u32, Trap> {
        let key = caller.read_memory(key32_offset, 32)?.to_vec();
        let is_cold = match Self::fn_impl(caller.data_mut(), &key, field) {
            Some((value, is_cold)) => {
                caller.write_memory(output32_offset, &value)?;
                is_cold
            }
            None => true,
        };
        Ok(is_cold as u32)
    }

    pub fn fn_impl<DB: IJournaledTrie>(
        ctx: &mut RuntimeContext<DB>,
        key: &[u8],
        field: u32,
    ) -> Option<([u8; 32], bool)> {
        let (field_values, _flags, is_cold) = ctx.jzkt().get(key.try_into().unwrap())?;
        let field_value = field_values.get(field as usize)?;
        if field_value.len() < 32 {
            return None;
        }
        let mut output = [0u8; 32];
        output.copy_from_slice(&field_value[0..32]);
        Some((output, is_cold))
    }
}
