use rwasm::{core::Trap, Caller};
use std::cell::Cell;

use fluentbase_types::IJournaledTrie;

use crate::RuntimeContext;

pub struct DebugLog;

thread_local! {
    pub static LAST_LOG_TIME: Cell<i64> = const { Cell::new(1) };
}

impl DebugLog {
    pub fn fn_handler<DB: IJournaledTrie>(
        caller: Caller<'_, RuntimeContext<DB>>,
        msg_offset: u32,
        msg_len: u32,
    ) -> Result<(), Trap> {
        let msg = caller.read_memory(msg_offset, msg_len)?;
        Self::fn_impl(msg);
        Ok(())
    }

    pub fn fn_impl(msg: &[u8]) {
        let now = chrono::offset::Utc::now();
        let last_time = LAST_LOG_TIME.get();
        let curr_time = now.timestamp_millis();
        let time_diff = if last_time > 0 {
            curr_time - last_time
        } else {
            0
        };
        LAST_LOG_TIME.set(curr_time);
        // let now_str = now.format("%Y%m%d_%H%M%S%.3f");
        println!(
            "(diff {}ms) debug_log: {}",
            time_diff,
            std::str::from_utf8(msg)
                .map(|s| s.to_string())
                .unwrap_or_else(|_| { hex::encode(msg) })
        );
    }
}
