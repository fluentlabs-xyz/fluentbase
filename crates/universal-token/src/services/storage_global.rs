use crate::services::storage::{StorageService, STORAGE_SERVICE_QUERY_CAP};
use fluentbase_sdk::syscall::{SYSCALL_ID_STORAGE_READ, SYSCALL_ID_STORAGE_WRITE};
use fluentbase_sdk::{debug_log, SyscallInvocationParams, B256, STATE_MAIN, U256};
use spin::{Mutex, MutexGuard};

pub static STORAGE_SERVICE: spin::Once<Mutex<StorageService>> = spin::Once::new();

pub fn storage_service<'a>(default_on_read: bool) -> MutexGuard<'a, StorageService> {
    let v = STORAGE_SERVICE.call_once(|| {
        let service = StorageService::new(default_on_read);
        Mutex::new(service)
    });
    v.lock()
}

pub const SLOT_QUERY_ELEM_LEN: usize = B256::len_bytes() + U256::BYTES;
pub const QUERY_BATCH_SPAN_LEN: usize = {
    assert!(STORAGE_SERVICE_QUERY_CAP > 0 && STORAGE_SERVICE_QUERY_CAP <= u8::MAX as usize);
    const V: usize = 1 + STORAGE_SERVICE_QUERY_CAP * SLOT_QUERY_ELEM_LEN; // +1 is for elements count
    V
};
static QUERY_BATCH_SPAN: spin::Once<Mutex<[u8; QUERY_BATCH_SPAN_LEN]>> = spin::Once::new();

pub fn lock_query_batch_span<'a>() -> MutexGuard<'a, [u8; QUERY_BATCH_SPAN_LEN]> {
    let state = QUERY_BATCH_SPAN.call_once(|| {
        let v = [0u8; QUERY_BATCH_SPAN_LEN];
        Mutex::new(v)
    });
    debug_assert!(
        !state.is_locked(),
        "evm: spin mutex is locked, looks like memory corruption"
    );
    state.lock()
}

pub fn query_batch_span_ptr() -> usize {
    lock_query_batch_span().as_slice().as_ptr() as usize
}

/// return query span memory offset
pub fn prepare_query_batch<const READ: bool, const DEFAULT_ON_READ: bool>(
) -> Option<SyscallInvocationParams> {
    let mut storage_service = storage_service(DEFAULT_ON_READ);
    let count = if READ {
        storage_service.keys_to_query().len()
    } else {
        storage_service.values_new().len()
    };
    if count <= 0 {
        return None;
    } else if count > 1 {
        debug_log!("count>1 not supported yet");
        return None;
    }
    let mut span = lock_query_batch_span();
    let span_mut = span.as_mut();
    let mut offset = 0;
    span_mut[offset] = count as u8;
    offset += 1;
    if READ {
        debug_log!();
        let slot = storage_service.keys_to_query_pop();
        if let Some(slot) = slot {
            span_mut[offset..offset + B256::len_bytes()].copy_from_slice(slot.as_le_slice());
            let ptr = span_mut.as_ptr() as usize;
            return Some(SyscallInvocationParams {
                code_hash: SYSCALL_ID_STORAGE_READ,
                input: ptr + offset..ptr + offset + B256::len_bytes(),
                fuel_limit: u64::MAX,
                state: STATE_MAIN,
                fuel16_ptr: 0,
            });
        };
    } else {
        debug_log!();
        if let Some((slot, value)) = storage_service.values_new_pop() {
            span_mut[offset..offset + B256::len_bytes()].copy_from_slice(slot.as_le_slice());
            span_mut[offset + B256::len_bytes()..offset + B256::len_bytes() + U256::BYTES]
                .copy_from_slice(&value.as_le_slice());
            let ptr = query_batch_span_ptr();
            return Some(SyscallInvocationParams {
                code_hash: SYSCALL_ID_STORAGE_WRITE,
                input: ptr + offset..ptr + offset + B256::len_bytes(),
                fuel_limit: u64::MAX,
                state: STATE_MAIN,
                fuel16_ptr: 0,
            });
        };
    }
    None
}

pub fn get_slot_key_at(idx: usize) -> U256 {
    let mut span = lock_query_batch_span();
    let span = span.as_slice();
    let offset = 1 + idx * U256::BYTES;
    U256::from_le_slice(&span[offset..offset + U256::BYTES])
}

pub fn print_stats() {
    let s = storage_service(false);
    debug_log!(
        "storage_service stats: {} {} {}",
        s.keys_to_query().len(),
        s.values_new().len(),
        s.values_existing().len()
    );
}
