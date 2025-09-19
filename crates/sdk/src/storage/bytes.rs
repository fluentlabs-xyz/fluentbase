use crate::{
    keccak256,
    storage::{StorageDescriptor, StorageLayout, StorageOps},
    B256, U256,
};
use alloc::{string::String, vec::Vec};
use core::marker::PhantomData;
use fluentbase_types::StorageAPI;

/// Dynamic byte array in storage.
/// Optimized for Solidity compatibility:
/// - Short (< 32 bytes): data and length*2 in base slot
/// - Long (≥ 32 bytes): length*2+1 in base slot, data at keccak256(base_slot)
#[derive(Debug, PartialEq, Eq)]
pub struct StorageBytes {
    base_slot: U256,
    _marker: PhantomData<()>,
}

impl Clone for StorageBytes {
    fn clone(&self) -> Self {
        *self
    }
}

impl Copy for StorageBytes {}

impl StorageBytes {
    pub const fn new(base_slot: U256) -> Self {
        Self {
            base_slot,
            _marker: PhantomData,
        }
    }

    /// Storage slot for long form data.
    fn data_slot(&self) -> U256 {
        let hash = keccak256(self.base_slot.to_be_bytes::<32>());
        U256::from_be_bytes(hash.0)
    }

    /// Read length and check if long form.
    fn read_metadata<S: StorageAPI>(&self, sdk: &S) -> (usize, bool) {
        let word = sdk.sload(self.base_slot);
        let last_byte = word.0[31];

        if last_byte & 1 == 0 {
            // Short form
            ((last_byte / 2) as usize, false)
        } else {
            // Long form
            let len = U256::from_be_bytes(word.0) / U256::from(2);
            (len.try_into().unwrap_or(0), true)
        }
    }

    /// Get byte array length.
    pub fn len<S: StorageAPI>(&self, sdk: &S) -> usize {
        self.read_metadata(sdk).0
    }

    /// Load entire byte array.
    pub fn load<S: StorageAPI>(&self, sdk: &S) -> Vec<u8> {
        let (len, is_long) = self.read_metadata(sdk);
        let mut result = Vec::with_capacity(len);

        if !is_long {
            // Short form
            let word = sdk.sload(self.base_slot);
            result.extend_from_slice(&word.0[..len]);
        } else {
            // Long form
            let data_base = self.data_slot();
            for i in 0..(len + 31) / 32 {
                let slot = data_base + U256::from(i);
                let word = sdk.sload(slot);
                let start = i * 32;
                let end = (start + 32).min(len);
                result.extend_from_slice(&word.0[..(end - start)]);
            }
        }

        result
    }

    /// Store byte array, replacing existing data.
    pub fn store<S: StorageAPI>(&self, sdk: &mut S, data: &[u8]) {
        let new_len = data.len();
        let (old_len, was_long) = self.read_metadata(sdk);

        // Clear old long form data if needed
        if was_long && old_len > 31 {
            let data_base = self.data_slot();
            for i in 0..(old_len + 31) / 32 {
                sdk.sstore(data_base + U256::from(i), B256::ZERO);
            }
        }

        if new_len < 32 {
            // Store as short form
            let mut word = B256::ZERO;
            word.0[..new_len].copy_from_slice(data);
            word.0[31] = (new_len * 2) as u8;
            sdk.sstore(self.base_slot, word);
        } else {
            // Store as long form
            let data_base = self.data_slot();

            // Write data chunks
            for (i, chunk) in data.chunks(32).enumerate() {
                let mut word = B256::ZERO;
                word.0[..chunk.len()].copy_from_slice(chunk);
                sdk.sstore(data_base + U256::from(i), word);
            }

            // Write length
            let len_encoded = U256::from(new_len * 2 + 1);
            sdk.sstore(self.base_slot, B256::from(len_encoded.to_be_bytes::<32>()));
        }
    }

    /// Read slice of bytes without modifying storage.
    pub fn slice<S: StorageAPI>(&self, sdk: &S, start: usize, end: usize) -> Vec<u8> {
        let data = self.load(sdk);
        let actual_end = end.min(data.len());
        data.get(start..actual_end)
            .map(|s| s.to_vec())
            .unwrap_or_default()
    }

    /// Append bytes to existing data.
    pub fn append<S: StorageAPI>(&self, sdk: &mut S, data: &[u8]) {
        let mut current = self.load(sdk);
        current.extend_from_slice(data);
        self.store(sdk, &current);
    }

    /// Truncate to specified length.
    pub fn truncate<S: StorageAPI>(&self, sdk: &mut S, new_len: usize) {
        let current = self.load(sdk);
        if new_len < current.len() {
            self.store(sdk, &current[..new_len]);
        }
    }

    /// Clear all data.
    pub fn clear<S: StorageAPI>(&self, sdk: &mut S) {
        let (len, is_long) = self.read_metadata(sdk);

        if is_long && len > 31 {
            let data_base = self.data_slot();
            for i in 0..(len + 31) / 32 {
                sdk.sstore(data_base + U256::from(i), B256::ZERO);
            }
        }

        sdk.sstore(self.base_slot, B256::ZERO);
    }
}

impl StorageDescriptor for StorageBytes {
    fn new(slot: U256, offset: u8) -> Self {
        debug_assert_eq!(offset, 0, "bytes always start at slot boundary");
        Self::new(slot)
    }

    fn slot(&self) -> U256 {
        self.base_slot
    }

    fn offset(&self) -> u8 {
        0
    }
}

impl StorageLayout for StorageBytes {
    type Descriptor = Self;
    type Accessor = Self;

    const BYTES: usize = 32;
    const SLOTS: usize = 1;

    fn access(descriptor: Self::Descriptor) -> Self::Accessor {
        descriptor
    }
}

/// UTF-8 string in storage.
#[derive(Debug, PartialEq, Eq)]
pub struct StorageString {
    base_slot: U256,
    _marker: PhantomData<()>,
}

impl Clone for StorageString {
    fn clone(&self) -> Self {
        *self
    }
}

impl Copy for StorageString {}

impl StorageString {
    pub const fn new(base_slot: U256) -> Self {
        Self {
            base_slot,
            _marker: PhantomData,
        }
    }

    /// Get string value.
    pub fn get<S: StorageAPI>(&self, sdk: &S) -> String {
        let bytes = self.as_bytes().load(sdk);
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// Set string value.
    pub fn set<S: StorageAPI>(&self, sdk: &mut S, value: &str) {
        self.as_bytes().store(sdk, value.as_bytes());
    }

    /// Get string length in bytes.
    pub fn len<S: StorageAPI>(&self, sdk: &S) -> usize {
        self.as_bytes().len(sdk)
    }

    /// Clear string.
    pub fn clear<S: StorageAPI>(&self, sdk: &mut S) {
        self.as_bytes().clear(sdk);
    }

    /// Convert to bytes for complex operations.
    pub fn as_bytes(&self) -> StorageBytes {
        StorageBytes::new(self.base_slot)
    }
}

impl StorageDescriptor for StorageString {
    fn new(slot: U256, offset: u8) -> Self {
        debug_assert_eq!(offset, 0, "strings always start at slot boundary");
        Self::new(slot)
    }

    fn slot(&self) -> U256 {
        self.base_slot
    }

    fn offset(&self) -> u8 {
        0
    }
}

impl StorageLayout for StorageString {
    type Descriptor = Self;
    type Accessor = Self;

    const BYTES: usize = 32;
    const SLOTS: usize = 1;

    fn access(descriptor: Self::Descriptor) -> Self::Accessor {
        descriptor
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::mock::MockStorage;

    #[test]
    fn test_bytes_basic_operations() {
        let mut sdk = MockStorage::new();
        let bytes = StorageBytes::new(U256::from(100));

        // Store and load
        let data = vec![0x11, 0x22, 0x33];
        bytes.store(&mut sdk, &data);
        assert_eq!(bytes.load(&sdk), data);
        assert_eq!(bytes.len(&sdk), 3);

        // Append
        bytes.append(&mut sdk, &[0x44, 0x55]);
        assert_eq!(bytes.load(&sdk), vec![0x11, 0x22, 0x33, 0x44, 0x55]);

        // Slice
        assert_eq!(bytes.slice(&sdk, 1, 4), vec![0x22, 0x33, 0x44]);
        assert!(bytes.slice(&sdk, 10, 20).is_empty()); // Out of bounds

        // Truncate
        bytes.truncate(&mut sdk, 3);
        assert_eq!(bytes.load(&sdk), vec![0x11, 0x22, 0x33]);

        // Clear
        bytes.clear(&mut sdk);
        assert_eq!(bytes.len(&sdk), 0);
    }

    #[test]
    fn test_bytes_form_transitions() {
        let mut sdk = MockStorage::new();
        let bytes = StorageBytes::new(U256::from(200));

        // Start with short form (31 bytes max)
        let short_data: Vec<u8> = (0..31).collect();
        bytes.store(&mut sdk, &short_data);

        // Verify short form storage layout
        let mut expected = String::new();
        for i in 0..31 {
            expected.push_str(&format!("{i:02x}"));
        }
        expected.push_str("3e"); // 31*2 = 62 = 0x3e
        assert_eq!(sdk.get_slot_hex(U256::from(200)), expected);

        // Append to trigger long form (32+ bytes)
        bytes.append(&mut sdk, &[31]);
        assert_eq!(bytes.len(&sdk), 32);

        // Verify long form: base slot has length*2+1 = 65 = 0x41
        assert_eq!(
            sdk.get_slot_hex(U256::from(200)),
            "0000000000000000000000000000000000000000000000000000000000000041"
        );

        // Verify data at keccak256(base_slot)
        let data_slot = {
            let hash = keccak256(U256::from(200).to_be_bytes::<32>());
            U256::from_be_bytes(hash.0)
        };
        let data = sdk.get_slot_hex(data_slot);
        assert_eq!(&data[0..2], "00"); // First byte is 0
        assert_eq!(&data[62..64], "1f"); // Last byte is 31

        // Truncate back to short form
        bytes.truncate(&mut sdk, 30);
        assert_eq!(bytes.len(&sdk), 30);

        // Verify short form restored and data slot cleared
        assert_eq!(sdk.get_slot_hex(data_slot), "0".repeat(64));
    }

    #[test]
    fn test_string_operations() {
        let mut sdk = MockStorage::new();
        let string = StorageString::new(U256::from(300));

        // Short string
        string.set(&mut sdk, "Hello");
        assert_eq!(string.get(&sdk), "Hello");
        assert_eq!(string.len(&sdk), 5);

        // Verify hex storage
        let hex = sdk.get_slot_hex(U256::from(300));
        assert_eq!(&hex[0..10], "48656c6c6f"); // "Hello" in hex
        assert_eq!(&hex[62..64], "0a"); // length*2 = 10

        // Long string (60 chars > 31 bytes)
        let long_str = "This is a very long string that exceeds 31 bytes threshold!!";
        string.set(&mut sdk, long_str);
        assert_eq!(string.get(&sdk), long_str);
        assert_eq!(string.len(&sdk), 60); // Actual length is 60 chars

        // Verify long form encoding
        let len_encoded = 60 * 2 + 1; // 121 = 0x79
        assert_eq!(sdk.get_slot(U256::from(300)), U256::from(len_encoded));

        // Complex operations via as_bytes()
        let bytes = string.as_bytes();
        let slice = bytes.slice(&sdk, 0, 4);
        assert_eq!(slice, b"This");

        bytes.append(&mut sdk, b" More text.");
        assert_eq!(string.len(&sdk), 71); // 60 + 11 = 71
        assert_eq!(
            string.get(&sdk),
            "This is a very long string that exceeds 31 bytes threshold!! More text."
        );

        // Clear
        string.clear(&mut sdk);
        assert_eq!(string.len(&sdk), 0);
    }

    #[test]
    fn test_bytes_multiple_slots() {
        let mut sdk = MockStorage::new();
        let bytes = StorageBytes::new(U256::from(400));

        // Store 100 bytes (requires 4 slots in long form)
        let data: Vec<u8> = (0..100).collect();
        bytes.store(&mut sdk, &data);

        assert_eq!(bytes.load(&sdk), data);
        assert_eq!(bytes.len(&sdk), 100);

        // Test slicing across slot boundaries
        let slice = bytes.slice(&sdk, 30, 35);
        assert_eq!(slice, vec![30, 31, 32, 33, 34]);

        // Append more data
        bytes.append(&mut sdk, &[100, 101, 102]);
        assert_eq!(bytes.len(&sdk), 103);

        // Truncate to exactly 32 bytes (still long form)
        bytes.truncate(&mut sdk, 32);
        assert_eq!(bytes.len(&sdk), 32);

        // Verify still in long form
        let base_slot_value = sdk.get_slot(U256::from(400));
        assert_eq!(base_slot_value, U256::from(65)); // 32*2+1 = 65
    }

    #[test]
    fn test_edge_cases() {
        let mut sdk = MockStorage::new();
        let bytes = StorageBytes::new(U256::from(500));

        // Empty operations
        assert!(bytes.slice(&sdk, 0, 10).is_empty());
        bytes.truncate(&mut sdk, 100); // No-op when empty
        assert_eq!(bytes.len(&sdk), 0);

        // Single byte
        bytes.store(&mut sdk, &[0xFF]);
        assert_eq!(bytes.load(&sdk), vec![0xFF]);

        // Exactly 31 bytes (max short)
        let data_31: Vec<u8> = vec![0xAA; 31];
        bytes.store(&mut sdk, &data_31);
        assert_eq!(bytes.len(&sdk), 31);

        // Exactly 32 bytes (min long)
        let data_32: Vec<u8> = vec![0xBB; 32];
        bytes.store(&mut sdk, &data_32);
        assert_eq!(bytes.len(&sdk), 32);

        // Verify it's in long form
        let base_value = sdk.get_slot(U256::from(500));
        assert_eq!(base_value, U256::from(65)); // 32*2+1
    }
}
