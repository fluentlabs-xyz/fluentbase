use crate::RuntimeContext;
use fluentbase_rwasm::{common::Trap, Caller};

pub struct ZkTrieField;

impl ZkTrieField {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        key32_offset: u32,
        field: u32,
        output32_offset: u32,
    ) -> Result<(), Trap> {
        let key = caller.read_memory(key32_offset, 32).to_vec();
        let output = Self::fn_impl(caller.data_mut(), &key, field);
        if let Some(output) = output {
            caller.write_memory(output32_offset, &output);
        }
        Ok(())
    }

    pub fn fn_impl<T>(context: &mut RuntimeContext<T>, key: &[u8], field: u32) -> Option<[u8; 32]> {
        let zktrie = context.zktrie.clone().unwrap();
        let field_values = zktrie.borrow().get(key)?;
        let field_value = field_values.get(field as usize)?;
        if field_value.len() < 32 {
            return None;
        }
        let mut output = [0u8; 32];
        output.copy_from_slice(&field_value[0..32]);
        Some(output)
    }
}
