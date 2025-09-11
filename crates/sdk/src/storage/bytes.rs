use crate::{
    keccak256,
    storage::{BytesAccess, StorageDescriptor, StorageLayout, StorageOps},
    B256, U256,
};
use alloc::{string::String, vec::Vec};
use core::marker::PhantomData;
use fluentbase_types::StorageAPI;

// --- 1. Bytes Descriptor ---

/// A descriptor for dynamic byte arrays in storage.
/// Follows Solidity's bytes/string storage optimization:
/// - Short (< 32 bytes): data and length*2 stored in base slot
/// - Long (≥ 32 bytes): length*2+1 in base slot, data at keccak256(base_slot)
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct StorageBytes {
    base_slot: U256,
    _marker: PhantomData<()>,
}

impl StorageBytes {
    /// Creates a new bytes descriptor at the given base slot.
    pub const fn new(base_slot: U256) -> Self {
        Self {
            base_slot,
            _marker: PhantomData,
        }
    }

    /// Returns the storage slot for long byte's data.
    fn data_slot(&self) -> U256 {
        let hash = keccak256(self.base_slot.to_be_bytes::<32>());
        U256::from_be_bytes(hash.0)
    }

    /// Reads the length and determines storage layout.
    /// Returns (length, is_long_form).
    fn read_length<S: StorageAPI>(&self, sdk: &S) -> (usize, bool) {
        let word = sdk.sload(self.base_slot);
        let last_byte = word.0[31];

        if last_byte & 1 == 0 {
            // Short form: length = last_byte / 2
            ((last_byte / 2) as usize, false)
        } else {
            // Long form: length = (word / 2)
            let word_value = U256::from_be_bytes(word.0);
            let len = word_value / U256::from(2);
            (len.try_into().unwrap_or(0), true)
        }
    }

    /// Writes the length to storage.
    fn write_length<S: StorageAPI>(&self, sdk: &mut S, len: usize) {
        if len < 32 {
            // Short form: store length * 2 in the last byte
            let mut word = sdk.sload(self.base_slot);
            word.0[31] = (len * 2) as u8;
            sdk.sstore(self.base_slot, word);
        } else {
            // Long form: store length * 2 + 1
            let len_encoded = U256::from(len * 2 + 1);
            sdk.sstore(self.base_slot, B256::from(len_encoded.to_be_bytes::<32>()));
        }
    }
}

impl StorageDescriptor for StorageBytes {
    fn new(slot: U256, offset: u8) -> Self {
        debug_assert_eq!(offset, 0, "bytes always start at slot boundary");
        Self {
            base_slot: slot,
            _marker: PhantomData,
        }
    }

    fn slot(&self) -> U256 {
        self.base_slot
    }

    fn offset(&self) -> u8 {
        0
    }
}

// --- 2. BytesAccess Implementation ---

impl BytesAccess for StorageBytes {
    fn len<S: StorageAPI>(&self, sdk: &S) -> usize {
        self.read_length(sdk).0
    }

    fn is_empty<S: StorageAPI>(&self, sdk: &S) -> bool {
        self.len(sdk) == 0
    }

    fn get<S: StorageAPI>(&self, sdk: &S, index: usize) -> u8 {
        let (len, is_long) = self.read_length(sdk);
        assert!(index < len, "bytes index out of bounds");

        if !is_long {
            // Short form: read from the base slot
            sdk.sload(self.base_slot).0[index]
        } else {
            // Long form: read from data slots
            let slot = self.data_slot() + U256::from(index / 32);
            sdk.sload(slot).0[index % 32]
        }
    }

    fn push<S: StorageAPI>(&self, sdk: &mut S, byte: u8) {
        let (old_len, _) = self.read_length(sdk);
        let new_len = old_len + 1;

        if old_len < 31 {
            // Still short after adding a byte
            let mut word = sdk.sload(self.base_slot);
            word.0[old_len] = byte;
            word.0[31] = (new_len * 2) as u8;
            sdk.sstore(self.base_slot, word);
        } else if old_len == 31 {
            // Converting from short to long form
            let mut word = sdk.sload(self.base_slot);
            word.0[31] = byte;

            // Copy data to long storage
            sdk.sstore(self.data_slot(), word);

            // Update length to long form
            self.write_length(sdk, new_len);
        } else {
            // Already long form
            let slot = self.data_slot() + U256::from(old_len / 32);
            let mut word = sdk.sload(slot);
            word.0[old_len % 32] = byte;
            sdk.sstore(slot, word);

            self.write_length(sdk, new_len);
        }
    }

    fn pop<S: StorageAPI>(&self, sdk: &mut S) -> Option<u8> {
        let (len, is_long) = self.read_length(sdk);
        if len == 0 {
            return None;
        }

        let index = len - 1;
        let byte = self.get(sdk, index);

        if len == 32 && is_long {
            // Converting from long to short form
            let mut word = sdk.sload(self.data_slot());
            word.0[31] = (index * 2) as u8;
            sdk.sstore(self.base_slot, word);

            // Clear the data slot
            sdk.sstore(self.data_slot(), B256::ZERO);
        } else if !is_long {
            // Short form
            let mut word = sdk.sload(self.base_slot);
            word.0[index] = 0;
            word.0[31] = (index * 2) as u8;
            sdk.sstore(self.base_slot, word);
        } else {
            // Long form - just update length
            self.write_length(sdk, index);

            // Clear the slot if it's now empty
            if index % 32 == 0 && index > 0 {
                let slot = self.data_slot() + U256::from(index / 32);
                sdk.sstore(slot, B256::ZERO);
            }
        }

        Some(byte)
    }

    fn load<S: StorageAPI>(&self, sdk: &S) -> Vec<u8> {
        let (len, is_long) = self.read_length(sdk);
        let mut result = Vec::with_capacity(len);

        if !is_long {
            // Short form: read from the base slot
            let word = sdk.sload(self.base_slot);
            result.extend_from_slice(&word.0[..len]);
        } else {
            // Long form: read from data slots
            let data_base = self.data_slot();
            for chunk_start in (0..len).step_by(32) {
                let slot = data_base + U256::from(chunk_start / 32);
                let word = sdk.sload(slot);
                let chunk_end = (chunk_start + 32).min(len);
                result.extend_from_slice(&word.0[..(chunk_end - chunk_start)]);
            }
        }

        result
    }

    fn store<S: StorageAPI>(&self, sdk: &mut S, bytes: &[u8]) {
        let new_len = bytes.len();
        let (old_len, was_long) = self.read_length(sdk);

        // Clear old long-form data if necessary
        if was_long && old_len > 31 {
            let data_base = self.data_slot();
            for i in 0..old_len.div_ceil(32) {
                sdk.sstore(data_base + U256::from(i), B256::ZERO);
            }
        }

        if new_len < 32 {
            // Store in short form
            let mut word = B256::ZERO;
            word.0[..new_len].copy_from_slice(bytes);
            word.0[31] = (new_len * 2) as u8;
            sdk.sstore(self.base_slot, word);
        } else {
            // Store in long form
            let data_base = self.data_slot();

            // Write data in 32-byte chunks
            for (i, chunk) in bytes.chunks(32).enumerate() {
                let mut word = B256::ZERO;
                word.0[..chunk.len()].copy_from_slice(chunk);
                sdk.sstore(data_base + U256::from(i), word);
            }

            // Write length
            self.write_length(sdk, new_len);
        }
    }

    fn clear<S: StorageAPI>(&self, sdk: &mut S) {
        let (len, is_long) = self.read_length(sdk);

        // Clear data slots if in long form
        if is_long && len > 31 {
            let data_base = self.data_slot();
            for i in 0..len.div_ceil(32) {
                sdk.sstore(data_base + U256::from(i), B256::ZERO);
            }
        }

        // Clear base slot (sets length to 0)
        sdk.sstore(self.base_slot, B256::ZERO);
    }
}

// --- 3. StorageLayout Implementation ---

impl StorageLayout for StorageBytes {
    type Descriptor = StorageBytes;
    type Accessor = Self;

    const REQUIRED_SLOTS: usize = 1;
    const ENCODED_SIZE: usize = 32;

    fn access(descriptor: Self::Descriptor) -> Self::Accessor {
        descriptor
    }
}

// --- 4. StorageString - Simple wrapper around Bytes ---

/// A descriptor for UTF-8 strings in storage.
/// Simple wrapper around Bytes with string-specific convenience methods.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct StorageString {
    bytes: StorageBytes,
}

impl StorageString {
    /// Creates a new string descriptor at the given base slot.
    pub const fn new(base_slot: U256) -> Self {
        Self {
            bytes: StorageBytes::new(base_slot),
        }
    }

    /// Loads the string from storage.
    pub fn get_string<S: StorageAPI>(&self, sdk: &S) -> String {
        let bytes = self.bytes.load(sdk);
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// Stores a string to storage.
    pub fn set_string<S: StorageAPI>(&self, sdk: &mut S, value: &str) {
        self.bytes.store(sdk, value.as_bytes());
    }

    /// Appends a string slice.
    pub fn push_str<S: StorageAPI>(&self, sdk: &mut S, string: &str) {
        for byte in string.bytes() {
            self.bytes.push(sdk, byte);
        }
    }
}

impl StorageDescriptor for StorageString {
    fn new(slot: U256, offset: u8) -> Self {
        debug_assert_eq!(offset, 0, "strings always start at slot boundary");
        Self {
            bytes: StorageBytes::new(slot),
        }
    }

    fn slot(&self) -> U256 {
        self.bytes.slot()
    }

    fn offset(&self) -> u8 {
        self.bytes.offset()
    }
}

// Delegate BytesAccess to inner Bytes
impl BytesAccess for StorageString {
    fn len<S: StorageAPI>(&self, sdk: &S) -> usize {
        self.bytes.len(sdk)
    }

    fn is_empty<S: StorageAPI>(&self, sdk: &S) -> bool {
        self.bytes.is_empty(sdk)
    }

    fn get<S: StorageAPI>(&self, sdk: &S, index: usize) -> u8 {
        self.bytes.get(sdk, index)
    }

    fn push<S: StorageAPI>(&self, sdk: &mut S, byte: u8) {
        self.bytes.push(sdk, byte);
    }

    fn pop<S: StorageAPI>(&self, sdk: &mut S) -> Option<u8> {
        self.bytes.pop(sdk)
    }

    fn load<S: StorageAPI>(&self, sdk: &S) -> Vec<u8> {
        self.bytes.load(sdk)
    }

    fn store<S: StorageAPI>(&self, sdk: &mut S, bytes: &[u8]) {
        self.bytes.store(sdk, bytes);
    }

    fn clear<S: StorageAPI>(&self, sdk: &mut S) {
        self.bytes.clear(sdk);
    }
}

impl StorageLayout for StorageString {
    type Descriptor = StorageString;
    type Accessor = Self;

    const REQUIRED_SLOTS: usize = 1;
    const ENCODED_SIZE: usize = 32;

    fn access(descriptor: Self::Descriptor) -> Self::Accessor {
        descriptor
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::mock::MockStorage;

    #[test]
    fn test_short_bytes_storage_layout() {
        let mut sdk = MockStorage::new();
        let bytes = StorageBytes::new(U256::from(100));

        // Test empty state
        assert_eq!(sdk.get_slot_hex(U256::from(100)), "0".repeat(64));

        // Add 3 bytes
        bytes.push(&mut sdk, 0x11);
        bytes.push(&mut sdk, 0x22);
        bytes.push(&mut sdk, 0x33);

        // Verify storage: data at the beginning, length*2 at the end
        let expected = "1122330000000000000000000000000000000000000000000000000000000006";
        assert_eq!(sdk.get_slot_hex(U256::from(100)), expected);

        // Verify API
        assert_eq!(bytes.len(&sdk), 3);
        assert_eq!(bytes.load(&sdk), vec![0x11, 0x22, 0x33]);
    }

    #[test]
    fn test_long_bytes_storage_layout() {
        let mut sdk = MockStorage::new();
        let bytes = StorageBytes::new(U256::from(200));

        // Push exactly 32 bytes
        let data: Vec<u8> = (0..32).collect();
        for &b in &data {
            bytes.push(&mut sdk, b);
        }

        // Verify the base slot contains length*2+1 = 65 = 0x41
        let expected_base = "0000000000000000000000000000000000000000000000000000000000000041";
        assert_eq!(sdk.get_slot_hex(U256::from(200)), expected_base);

        // Verify data is at keccak256(base_slot)
        let data_slot = {
            let hash = keccak256(U256::from(200).to_be_bytes::<32>());
            U256::from_be_bytes(hash.0)
        };

        let mut expected_data = String::new();
        for i in 0..32 {
            expected_data.push_str(&format!("{i:02x}"));
        }
        assert_eq!(sdk.get_slot_hex(data_slot), expected_data);
    }

    #[test]
    fn test_bytes_transition_short_to_long() {
        let mut sdk = MockStorage::new();
        let bytes = StorageBytes::new(U256::from(300));

        // Fill to 31 bytes (maximum short form)
        for i in 0..31 {
            bytes.push(&mut sdk, i as u8);
        }

        // Verify short form storage
        let mut expected = String::new();
        for i in 0..31 {
            expected.push_str(&format!("{i:02x}"));
        }
        expected.push_str("3e"); // 31*2 = 62 = 0x3e
        assert_eq!(sdk.get_slot_hex(U256::from(300)), expected);

        // Push one more byte to trigger the long form
        bytes.push(&mut sdk, 31);

        // Verify long form: base slot has length*2+1 = 65 = 0x41
        assert_eq!(
            sdk.get_slot_hex(U256::from(300)),
            "0000000000000000000000000000000000000000000000000000000000000041"
        );

        // Verify data moved to keccak256(base_slot)
        let data_slot = {
            let hash = keccak256(U256::from(300).to_be_bytes::<32>());
            U256::from_be_bytes(hash.0)
        };

        let mut expected_data = String::new();
        for i in 0..32 {
            expected_data.push_str(&format!("{i:02x}"));
        }
        assert_eq!(sdk.get_slot_hex(data_slot), expected_data);
    }

    #[test]
    fn test_bytes_transition_long_to_short() {
        let mut sdk = MockStorage::new();
        let bytes = StorageBytes::new(U256::from(400));

        // Create long form (32 bytes)
        for i in 0..32 {
            bytes.push(&mut sdk, i as u8);
        }

        let data_slot = {
            let hash = keccak256(U256::from(400).to_be_bytes::<32>());
            U256::from_be_bytes(hash.0)
        };

        // Verify long form
        assert_eq!(
            sdk.get_slot_hex(U256::from(400)),
            "0000000000000000000000000000000000000000000000000000000000000041"
        );

        // Pop one byte to trigger the short form
        assert_eq!(bytes.pop(&mut sdk), Some(31));

        // Verify short form restored
        let mut expected = String::new();
        for i in 0..31 {
            expected.push_str(&format!("{i:02x}"));
        }
        expected.push_str("3e"); // 31*2 = 62 = 0x3e
        assert_eq!(sdk.get_slot_hex(U256::from(400)), expected);

        // Verify data slot cleared
        assert_eq!(sdk.get_slot_hex(data_slot), "0".repeat(64));
    }

    #[test]
    fn test_bytes_store_and_clear() {
        let mut sdk = MockStorage::new();
        let bytes = StorageBytes::new(U256::from(500));

        // Store long data
        let data: Vec<u8> = (100..150).collect();
        bytes.store(&mut sdk, &data);

        // Verify length in base slot: 50*2+1 = 101 = 0x65
        assert_eq!(
            sdk.get_slot_hex(U256::from(500)),
            "0000000000000000000000000000000000000000000000000000000000000065"
        );

        // Verify data stored correctly
        assert_eq!(bytes.load(&sdk), data);

        // Store short data (should clear old slots)
        let short_data = vec![0xAA, 0xBB, 0xCC];
        bytes.store(&mut sdk, &short_data);

        // Verify short form
        assert_eq!(
            sdk.get_slot_hex(U256::from(500)),
            "aabbcc0000000000000000000000000000000000000000000000000000000006"
        );

        // Clear all
        bytes.clear(&mut sdk);
        assert_eq!(sdk.get_slot_hex(U256::from(500)), "0".repeat(64));
        assert!(bytes.is_empty(&sdk));
    }

    #[test]
    fn test_string_storage() {
        let mut sdk = MockStorage::new();
        let string = StorageString::new(U256::from(600));

        // Test short string
        string.set_string(&mut sdk, "Hello");
        assert_eq!(string.get_string(&sdk), "Hello");

        // Verify hex storage
        let hex = sdk.get_slot_hex(U256::from(600));
        assert_eq!(&hex[0..10], "48656c6c6f"); // "Hello" in hex
        assert_eq!(&hex[62..64], "0a"); // length*2 = 10

        // Test long string
        let long_str = "This is a very long string that will use long form storage!";
        string.set_string(&mut sdk, long_str);
        assert_eq!(string.get_string(&sdk), long_str);

        // Verify base slot has length encoded
        let len_encoded = long_str.len() * 2 + 1;
        assert_eq!(sdk.get_slot(U256::from(600)), U256::from(len_encoded));
    }

    #[test]
    #[should_panic(expected = "bytes index out of bounds")]
    fn test_bytes_bounds_check() {
        let sdk = MockStorage::new();
        let bytes = StorageBytes::new(U256::from(800));
        bytes.get(&sdk, 0); // Should panic - empty bytes
    }

    #[test]
    fn test_bytes_multiple_slots() {
        let mut sdk = MockStorage::new();
        let bytes = StorageBytes::new(U256::from(900));

        // Store 100 bytes (requires 4 slots)
        let data: Vec<u8> = (0..100).collect();
        bytes.store(&mut sdk, &data);

        // Verify all data stored and retrieved correctly
        assert_eq!(bytes.load(&sdk), data);
        assert_eq!(bytes.len(&sdk), 100);

        // Verify individual bytes
        for i in 0..100 {
            assert_eq!(bytes.get(&sdk, i), i as u8);
        }
    }
}
