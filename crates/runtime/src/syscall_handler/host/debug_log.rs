use crate::RuntimeContext;
use core::cell::Cell;
use rwasm::{Store, TrapCode, Value};

thread_local! {
    pub static LAST_LOG_TIME: Cell<u128> = const { Cell::new(0) };
}

pub fn syscall_debug_log_handler(
    caller: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    let (message_ptr, message_len) = (
        params[0].i32().unwrap() as usize,
        params[1].i32().unwrap() as usize,
    );
    let mut buffer = vec![0u8; message_len];
    caller.memory_read(message_ptr, &mut buffer)?;
    syscall_debug_log_impl(&buffer);
    Ok(())
}

#[cfg(feature = "debug-print")]
pub fn syscall_debug_log_impl(msg: &[u8]) {
    use std::time::SystemTime;
    let curr_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_micros();
    let last_time = LAST_LOG_TIME.get();
    let time_diff = if last_time > 0 {
        curr_time - last_time
    } else {
        0
    };
    LAST_LOG_TIME.set(curr_time);
    const MSG_LIMIT: usize = 256000;
    let msg = if msg.len() > MSG_LIMIT {
        &msg[..MSG_LIMIT]
    } else {
        &msg[..]
    };
    println!(
        "debug_log (diff {}us): {}",
        time_diff,
        std::str::from_utf8(msg)
            .map(|s| s.to_string())
            .unwrap_or("non utf-8 message".to_string())
    );
}

#[cfg(not(feature = "debug-print"))]
pub fn syscall_debug_log_impl(_msg: &[u8]) {}
