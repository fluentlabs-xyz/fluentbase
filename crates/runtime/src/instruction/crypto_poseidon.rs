use crate::RuntimeContext;
use rwasm::{common::Trap, Caller};

pub struct CryptoPoseidon;

impl CryptoPoseidon {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        f32s_offset: u32,
        f32s_len: u32,
        output_offset: u32,
    ) -> Result<(), Trap> {
        println!("DEBUG POSEIDON 1 WE CALL fn handler");
        let data = caller.read_memory(f32s_offset, f32s_len);
        println!("DEBUG WE DO MEMORY WRITE FOR POSEIDON 1 fn handler");
        caller.write_memory(output_offset, &Self::fn_impl(data));
        Ok(())
    }

    pub fn fn_impl(data: &[u8]) -> [u8; 32] {
        use fluentbase_poseidon::poseidon_hash;
        poseidon_hash(data)
    }
}
