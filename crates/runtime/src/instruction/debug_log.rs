use crate::RuntimeContext;
use core::cell::Cell;
use rwasm::{core::Trap, Caller};

pub struct SyscallDebugLog;

thread_local! {
    pub static LAST_LOG_TIME: Cell<i64> = const { Cell::new(0) };
}

impl SyscallDebugLog {
    pub fn fn_handler(
        caller: Caller<'_, RuntimeContext>,
        msg_offset: u32,
        msg_len: u32,
    ) -> Result<(), Trap> {
        let msg = caller.read_memory(msg_offset, msg_len)?;
        Self::fn_impl(msg);
        Ok(())
    }

    pub fn fn_impl(_msg: &[u8]) {
        // use std::time::Instant;
        // let now = Instant::now();
        // let last_time = LAST_LOG_TIME.get();
        // let curr_time = now.elapsed().as_millis() as i64;
        // let time_diff = if last_time > 0 {
        //     curr_time - last_time
        // } else {
        //     0
        // };
        // LAST_LOG_TIME.set(curr_time);
        // // let now_str = now.format("%Y%m%d_%H%M%S%.3f");
        // let msg = if msg.len() > 1000 {
        //     &msg[..1000]
        // } else {
        //     &msg[..]
        // };
        // println!(
        //     "debug_log (diff {}ms): {}",
        //     time_diff,
        //     std::str::from_utf8(msg)
        //         .map(|s| s.to_string())
        //         .unwrap_or_else(|_| { hex::encode(msg) })
        // );
    }
}
