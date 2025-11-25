use alloc::vec;
use core::sync::atomic::{AtomicU64, Ordering};
use fluentbase_sdk::{debug_log, keccak256, B256, U256};

static GLOBAL_SLOT: AtomicU64 = AtomicU64::new(1);
const SLOT_BYTES: usize = size_of::<u64>();

pub trait IKeyDeriver {
    fn slot(&self) -> u64;
    fn b256(&self, v: &B256) -> U256;
    fn u256(&self, v: &U256) -> U256;
    fn slice(&self, v: &[u8]) -> U256;
}

pub struct KeyDeriver {
    slot: u64,
}

impl KeyDeriver {
    pub fn new() -> Self {
        let slot = GLOBAL_SLOT.fetch_add(1, Ordering::Relaxed);
        Self { slot }
    }
    /// use this to enforce specific slot usage
    pub fn new_specific_slot(slot: u64) -> Self {
        let current_slot = GLOBAL_SLOT.fetch_add(1, Ordering::Relaxed);
        if slot != current_slot {
            debug_log!(
                "Slot {} is not equal to current slot {}",
                slot,
                current_slot
            );
            panic!("incorrect selected slot");
        }
        Self { slot }
    }
}

impl IKeyDeriver for KeyDeriver {
    fn slot(&self) -> u64 {
        self.slot
    }

    fn b256(&self, v: &B256) -> U256 {
        let mut data = [0u8; SLOT_BYTES + size_of::<B256>()];
        data[..SLOT_BYTES].copy_from_slice(self.slot.to_be_bytes().as_ref());
        data[SLOT_BYTES..].copy_from_slice(&v.0);
        keccak256(&data).into()
    }

    fn u256(&self, v: &U256) -> U256 {
        let mut data = [0u8; SLOT_BYTES + size_of::<B256>()];
        data[..SLOT_BYTES].copy_from_slice(self.slot.to_be_bytes().as_ref());
        data[SLOT_BYTES..].copy_from_slice(&v.as_le_bytes());
        keccak256(&data).into()
    }

    fn slice(&self, v: &[u8]) -> U256 {
        let mut data = vec![0u8; SLOT_BYTES + v.len()];
        data[..SLOT_BYTES].copy_from_slice(self.slot.to_be_bytes().as_ref());
        data[SLOT_BYTES..].copy_from_slice(v);
        keccak256(&data).into()
    }
}
