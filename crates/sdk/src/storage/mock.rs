#![allow(dead_code)]
use alloc::string::String;
use alloc::vec::Vec;
use fluentbase_types::{hex, syscall::SyscallResult, ExitCode, StorageAPI, U256};
use hashbrown::HashMap;

/// A mock implementation of `StorageAPI` that simulates contract storage
/// using an in-memory HashMap. It is `Clone`-able to easily create
/// separate instances for different testing phases or checks.
#[derive(Clone, Default, Debug)]
pub struct MockStorage {
    pub storage: HashMap<U256, U256>,
}

impl MockStorage {
    /// Creates a new, empty `MockSDK`.
    pub fn new() -> Self {
        Default::default()
    }

    /// Retrieves the raw `U256` value from a specific storage slot.
    /// Returns `U256::ZERO` if the slot has not been written to.
    pub fn get_slot(&self, slot: impl Into<U256>) -> U256 {
        self.storage.get(&slot.into()).copied().unwrap_or_default()
    }

    /// A convenience helper to get the raw slot content as a hex string.
    /// Useful for snapshot testing of packed data.
    pub fn get_slot_hex(&self, slot: impl Into<U256>) -> String {
        hex::encode(self.get_slot(slot).to_be_bytes::<32>())
    }

    /// A setup helper to directly initialize storage with raw data from a hex string.
    /// Panics if the hex string is invalid.
    pub fn init_slot(&mut self, slot: impl Into<U256>, hex_data: &str) -> &mut Self {
        let slot = slot.into();
        let stripped = hex_data.strip_prefix("0x").unwrap_or(hex_data);
        let bytes = hex::decode(stripped).expect("invalid hex string in init_slot");
        let value =
            U256::from_be_bytes::<32>(bytes.try_into().expect("hex string must be 32 bytes long"));
        self.storage.insert(slot, value);
        self
    }

    /// Returns all slots sorted by key
    pub fn sorted_slots(&self) -> Vec<(U256, U256)> {
        let mut slots: Vec<(U256, U256)> = self.storage.iter().map(|(k, v)| (*k, *v)).collect();
        slots.sort_by_key(|(slot, _)| *slot);
        slots
    }
}

// --- StorageAPI Implementation ---

impl StorageAPI for MockStorage {
    fn write_storage(&mut self, slot: U256, value: U256) -> SyscallResult<()> {
        if value == U256::ZERO {
            // Mimic EVM behavior: setting to zero is equivalent to deleting.
            self.storage.remove(&slot);
        } else {
            self.storage.insert(slot, value);
        }
        SyscallResult::new((), 0, 0, ExitCode::Ok)
    }

    fn storage(&self, slot: &U256) -> SyscallResult<U256> {
        let value = self.get_slot(*slot);
        SyscallResult::new(value, 0, 0, ExitCode::Ok)
    }
}
