use crate::{Bytes, SharedContextInputV1};
use alloc::{vec, vec::Vec};
use core::sync::atomic::{AtomicU32, Ordering};
use fluentbase_types::system::{
    JournalStorage, RuntimeInterruptionOutcomeV1, RuntimeNewFrameInputV1,
};
use spin::{Mutex, MutexGuard, Once};

pub(super) struct RecoverableState {
    pub(super) storage: JournalStorage,
    pub(super) metadata: Bytes,
    pub(super) input: Bytes,
    pub(super) context: SharedContextInputV1,
    // pub(super) balances: HashMap<Address, U256>,
    pub(super) output: Vec<u8>,
    pub(super) interruption_outcome: Option<RuntimeInterruptionOutcomeV1>,
    pub(super) unique_key: u32,
    pub(super) intermediary_input: Option<Bytes>,
}

impl RecoverableState {
    pub(super) fn new(input: RuntimeNewFrameInputV1) -> Self {
        let RuntimeNewFrameInputV1 {
            metadata,
            input,
            context,
            storage,
            ..
        } = input;
        let context = SharedContextInputV1::decode_from_slice(context.as_ref()).unwrap();
        Self {
            storage: JournalStorage::new(storage.unwrap_or_default()),
            metadata,
            input,
            context,
            output: vec![],
            interruption_outcome: None,
            unique_key: next_unique_key(),
            intermediary_input: None,
        }
    }

    pub(super) fn remember(self) {
        let mut recoverable_state = lock_state_context();
        recoverable_state.push(self);
    }

    pub(super) fn recover(outcome: RuntimeInterruptionOutcomeV1) -> Self {
        let mut recoverable_state = lock_state_context();
        let Some(mut state) = recoverable_state.pop() else {
            unreachable!("missing cached evm state, can't resume execution")
        };
        _ = state.interruption_outcome.insert(outcome);
        state
    }
}

pub(super) fn lock_state_context<'a>() -> MutexGuard<'a, Vec<RecoverableState>> {
    static SAVED_STATE_CONTEXT: Once<Mutex<Vec<RecoverableState>>> = Once::new();
    let cached_state = SAVED_STATE_CONTEXT.call_once(|| Mutex::new(Vec::new()));
    // The best we can do here is panic and fallback into `FatalExternalError`, because it means that
    //  the runtime state is corrupted, and it's much safer juts to crash. If mutex is locked, then we bypassed
    // the fatal handler somehow, or it's massive memory corruption (maybe because of shared memory or
    // incorrect system memory cleanup).
    if cached_state.is_locked() {
        unreachable!("spin mutex is locked, looks like runtime corruption");
    }
    cached_state.lock()
}

pub(super) fn next_unique_key() -> u32 {
    static NEXT_STATE_KEY: AtomicU32 = AtomicU32::new(0);
    // Safety: 1k TPS gives ~50 days of key rotation that seems to be safe enough, especially since we reset state every transaction
    NEXT_STATE_KEY.fetch_add(1, Ordering::Relaxed)
}
