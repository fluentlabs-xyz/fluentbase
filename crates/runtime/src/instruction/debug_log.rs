use rwasm::{core::Trap, Caller};

use fluentbase_types::IJournaledTrie;

use crate::RuntimeContext;

pub struct DebugLog;

impl DebugLog {
    pub fn fn_handler<DB: IJournaledTrie>(
        mut caller: Caller<'_, RuntimeContext<DB>>,
        msg_offset: u32,
        msg_len: u32,
    ) -> Result<(), Trap> {
        let msg = caller.read_memory(msg_offset, msg_len)?;
        Self::fn_impl(msg);
        Ok(())
    }

    pub fn fn_impl(msg: &[u8]) {
        let now = chrono::offset::Utc::now();
        let now_str = now.format("%Y%m%d_%H%M%S%.3f");
        match std::str::from_utf8(msg) {
            Ok(v) => {
                println!("{} debug: {}", now_str, v);
            }
            Err(v) => {
                println!("{} debug: failed to convert msg into utf8: {}", now_str, v);
            }
        };
    }
}
