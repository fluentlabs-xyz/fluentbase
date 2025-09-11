use crate::{B256, U256};
use alloc::vec::Vec;
use fluentbase_types::StorageAPI;

pub mod primitive;

pub mod array;
pub mod bytes;
pub mod map;

pub mod composite;
pub mod mock;
pub use mock::MockStorage;

pub mod vec;

/// The main trait that connects a Rust type to its storage layout and access API.
/// Any type that can be a field in a `#[fluent_storage]` struct must implement this.
pub trait StorageLayout: Sized {
    /// A lightweight, `Copy`-able struct describing the storage location.
    type Descriptor: StorageDescriptor;

    /// A temporary proxy object providing the interactive API for this type.
    type Accessor;

    /// The number of contiguous slots required by this type's layout.
    const REQUIRED_SLOTS: usize;

    const ENCODED_SIZE: usize;

    /// Creates an Accessor, the sole entry point for interacting with the stored value.
    fn access(descriptor: Self::Descriptor) -> Self::Accessor;
}

/// Trait that all storage descriptors must implement.
/// Provides a uniform interface for creating and accessing storage locations.
pub trait StorageDescriptor {
    /// Creates a descriptor at the specified storage location.
    /// For composite types (arrays, structs, maps), offset should be 0.
    /// For packed primitive types, offset indicates position within the slot.
    fn new(slot: U256, offset: u8) -> Self;

    /// Returns the base storage slot.
    fn slot(&self) -> U256;

    /// Returns the offset within the slot (0 for non-packed types).
    fn offset(&self) -> u8;
}

/// A trait for types that have a fixed-size byte representation suitable for storage.
///
/// This trait defines the low-level serialization and deserialization logic
/// for types that can be packed within a single 32-byte storage slot.
pub trait PrimitiveCodec: Sized {
    /// The exact number of bytes this type occupies when encoded. Must be <= 32.
    const ENCODED_SIZE: usize;

    /// Encodes the value into the provided byte slice.
    ///
    /// # Panics
    /// if the length of `target` is not equal to `ENCODED_SIZE`.
    fn encode_into(&self, target: &mut [u8]);

    /// Decodes a value from the provided byte slice.
    ///
    /// # Panics
    /// Panics if the length of `bytes` is not equal to `ENCODED_SIZE`.
    fn decode(bytes: &[u8]) -> Self;
}

/// ----------------------------------
/// Accessor traits
/// ----------------------------------
/// Defines the API for a single, primitive value that implements `StorageCodec`.
pub trait PrimitiveAccess<T: PrimitiveCodec> {
    /// Reads the value from storage.
    fn get<S: StorageAPI>(&self, sdk: &S) -> T;

    /// Writes a new value to storage.
    fn set<S: StorageAPI>(&self, sdk: &mut S, value: T);
}

/// Defines the API for a fixed-size array of `StorageLayout` elements.
pub trait ArrayAccess<T: StorageLayout, const N: usize> {
    /// Returns an accessor for the element at the given index.
    ///
    /// # Panics
    /// Panics if `index >= N`.
    fn at(&self, index: usize) -> T::Accessor;
}
/// Defines the API for a key-value map where values are `StorageLayout` types.
pub trait MapAccess<K: MapKey, V: StorageLayout> {
    /// Returns an accessor for the value at the given key.
    /// The accessor can be used to get or set the value.
    fn entry(&self, key: K) -> V::Accessor;
}

/// Defines the API for a dynamic vector of `StorageLayout` elements.
/// Dynamic vectors in Solidity store their length at the base slot,
/// and elements are stored starting at keccak256(base_slot).
pub trait VecAccess<T: StorageLayout> {
    /// Returns the number of elements in the vector.
    fn len<S: StorageAPI>(&self, sdk: &S) -> u64;

    /// Returns `true` if the vector is empty.
    fn is_empty<S: StorageAPI>(&self, sdk: &S) -> bool {
        self.len(sdk) == 0
    }

    /// Returns an accessor for the element at `index`.
    /// # Panics
    /// Panics if `index >= self.len()`.
    fn at(&self, index: u64) -> T::Accessor;

    /// Appends a new element and returns an accessor to initialize it.
    /// Updates the length and returns accessor to the new element.
    fn push<S: StorageAPI>(&self, sdk: &mut S) -> T::Accessor;

    /// Removes the last element by decrementing the length.
    /// Does not clear the storage slot (gas optimization).
    fn pop<S: StorageAPI>(&self, sdk: &mut S);

    /// Clears the vector by setting length to 0.
    /// Does not clear individual elements (gas optimization).
    fn clear<S: StorageAPI>(&self, sdk: &mut S);
}
/// Defines the specialized API for dynamic byte arrays.
/// Dynamic byte arrays in Solidity use optimized storage:
/// - Short (< 32 bytes): data and length stored inline
/// - Long (≥ 32 bytes): length at base slot, data at keccak256(base_slot)
pub trait BytesAccess {
    /// Returns the number of bytes.
    fn len<S: StorageAPI>(&self, sdk: &S) -> usize;

    /// Returns `true` if the array is empty.
    fn is_empty<S: StorageAPI>(&self, sdk: &S) -> bool {
        self.len(sdk) == 0
    }

    /// Reads a single byte at `index`.
    /// # Panics
    /// Panics if `index >= self.len()`.
    fn get<S: StorageAPI>(&self, sdk: &S, index: usize) -> u8;

    /// Appends a byte.
    fn push<S: StorageAPI>(&self, sdk: &mut S, byte: u8);

    /// Removes and returns the last byte, or `None` if empty.
    fn pop<S: StorageAPI>(&self, sdk: &mut S) -> Option<u8>;

    /// Reads all bytes into a `Vec<u8>`.
    /// Use with caution due to gas costs for large arrays.
    fn load<S: StorageAPI>(&self, sdk: &S) -> Vec<u8>;

    /// Overwrites the entire storage with new data.
    /// Efficiently handles transition between short and long forms.
    fn store<S: StorageAPI>(&self, sdk: &mut S, bytes: &[u8]);

    /// Clears all bytes by setting length to 0.
    /// May clear storage slots for gas optimization.
    fn clear<S: StorageAPI>(&self, sdk: &mut S);
}

/// ----------------------------------
/// Helper traits
/// ----------------------------------
/// A trait for types that can be used as keys in a `Map`.
pub trait MapKey {
    /// Computes the final storage slot for an item within a map.
    fn compute_slot(&self, root_slot: U256) -> U256;
}

/// An internal extension trait for `StorageAPI` providing low-level,
/// packed storage operations. Not intended for public use.
pub(crate) trait StorageOps: StorageAPI {
    /// Reads a 32-byte word from storage, returning a B256.
    /// Panics on syscall failure.
    fn sload(&self, slot: U256) -> B256 {
        self.storage(&slot).unwrap().into()
    }

    /// Writes a 32-byte word to storage.
    /// Panics on syscall failure.
    fn sstore(&mut self, slot: U256, value: B256) {
        self.write_storage(slot, value.into()).unwrap()
    }

    /// Reads a value of type `T` that implements `StorageCodec` from a specific
    /// location (slot + offset) within a storage word.
    fn read_at<T: PrimitiveCodec>(&self, slot: U256, offset: u8) -> T {
        // offset is from the LEFT edge (start of byte array)
        let start = offset as usize;
        let end = start + T::ENCODED_SIZE;

        assert!(end <= 32, "read out of slot bounds");

        let word = self.sload(slot);
        T::decode(&word[start..end])
    }

    /// Writes a value of type `T` that implements `StorageCodec` to a specific
    /// location (slot + offset) within a storage word.
    fn write_at<T: PrimitiveCodec>(&mut self, slot: U256, offset: u8, value: &T) {
        // offset is from the LEFT edge (start of byte array)
        let start = offset as usize;
        let end = start + T::ENCODED_SIZE;

        assert!(end <= 32, "write out of slot bounds");

        if T::ENCODED_SIZE == 32 && offset == 0 {
            let mut word_bytes = [0u8; 32];
            value.encode_into(&mut word_bytes);
            self.sstore(slot, B256::from(word_bytes));
            return;
        }

        let mut word = self.sload(slot);
        value.encode_into(&mut word.0[start..end]);
        self.sstore(slot, word);
    }
}

/// Automatically implement `StorageOps` for any type that already implements `StorageAPI`.
impl<S: StorageAPI> StorageOps for S {}
