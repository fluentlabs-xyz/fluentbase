//! Storage abstraction layer for Ethereum smart contracts.

use crate::{StorageAPI, B256, U256};

mod array;
mod bytes;
mod map;
mod primitive;
mod vec;

pub use array::*;
pub use bytes::*;
use fluentbase_types::ExitCode;
pub use map::*;
pub use primitive::*;
pub use vec::*;
// test utils
pub mod mock;

/// Trait connecting Rust types to Ethereum's storage model.
///
/// Ethereum storage is a key-value store with 2^256 slots, each holding 32 bytes.
/// This trait defines how Rust types map to these slots through two key concepts:
///
/// 1. **Storage Layout** - How much space a type needs:
///    - Primitive types < 32 bytes can be packed together in one slot
///    - Full-width types (32 bytes) occupy exactly one slot
///    - Complex types (arrays, structs) may span multiple slots
///
/// 2. **Two-phase Access Pattern**:
///    - **Descriptor**: Lightweight metadata about WHERE data lives (slot + offset)
///    - **Accessor**: The actual API for HOW to read/write that data
///
/// This separation allows zero-cost abstractions: descriptors are computed at
/// compile-time, while accessors provide type-safe runtime operations.
pub trait StorageLayout: Sized {
    /// Lightweight location metadata.
    ///
    /// For packable types: contains slot + byte offset
    /// For full types: contains only slot number
    /// Computed at compile-time, Copy + Send + Sync
    type Descriptor: StorageDescriptor + Copy;

    /// Runtime access interface.
    ///
    /// Different types have different accessors:
    /// - `Primitive<T>`: simple get/set operations
    /// - Array<T, N>: indexed access via at(index)
    /// - Map<K, V>: key-based access via entry(key)
    /// - `Vec<T>`: dynamic operations (push/pop/at)
    type Accessor;

    /// Size in bytes when encoded to storage.
    ///
    /// Used to determine:
    /// - Whether type can be packed (BYTES < 32)
    /// - Offset calculation for packed fields
    /// - Total storage consumption
    const BYTES: usize;

    /// Number of storage slots this type reserves.
    ///
    /// - 0: Type can be packed with others (bool, u8, u16, etc.)
    /// - 1: Type uses exactly one slot (u256, address for full slot)
    /// - N: Type uses N slots (arrays, structs, etc.)
    ///
    /// Note: SLOTS = 0 means the type doesn't reserve a full slot,
    /// allowing the compiler to pack multiple such types together.
    const SLOTS: usize;

    /// Construct accessor from descriptor.
    ///
    /// This is the entry point for all storage operations.
    /// The descriptor tells WHERE, the accessor provides HOW.
    fn access(descriptor: Self::Descriptor) -> Self::Accessor;
}

/// Uniform interface for creating storage descriptors at specific locations.
pub trait StorageDescriptor: Copy {
    fn new(slot: U256, offset: u8) -> Self;
    fn slot(&self) -> U256;
    fn offset(&self) -> u8;
}

/// Encoding/decoding for types that fit within a single storage slot.
///
/// Implementors must guarantee:
/// - ENCODED_SIZE <= 32
/// - encode_into/decode are inverse operations
/// - Big-endian encoding for consistency with EVM
pub trait PackableCodec: Sized + Copy {
    /// Exact size in bytes (must be <= 32).
    const ENCODED_SIZE: usize;

    /// Encode value to bytes at target position.
    fn encode_into(&self, target: &mut [u8]);

    /// Decode value from bytes.
    fn decode(bytes: &[u8]) -> Self;
}

/// Low-level storage operations.
///
/// Extension trait providing optimized read/write for packed values.
/// Automatically implemented for all types implementing StorageAPI.
pub(crate) trait StorageOps: StorageAPI {
    /// Read full 32-byte slot.
    fn sload(&self, slot: U256) -> Result<B256, ExitCode> {
        let result = self.storage(&slot).ok()?;
        Ok(result.into())
    }

    /// Write full 32-byte slot.
    fn sstore(&mut self, slot: U256, value: B256) -> Result<(), ExitCode> {
        self.write_storage(slot, value.into()).ok()
    }

    /// Read packed value from slot at specific offset.
    ///
    /// Optimization: single SLOAD even for small types.
    fn read_at<T: PackableCodec>(&self, slot: U256, offset: u8) -> Result<T, ExitCode> {
        let start = offset as usize;
        let end = start + T::ENCODED_SIZE;
        assert!(end <= 32, "read out of bounds");

        let word = self.sload(slot)?;
        Ok(T::decode(&word[start..end]))
    }

    /// Write packed value to slot at specific offset.
    ///
    /// Optimization: skip SLOAD for full-slot writes.
    fn write_at<T: PackableCodec>(
        &mut self,
        slot: U256,
        offset: u8,
        value: &T,
    ) -> Result<(), ExitCode> {
        let start = offset as usize;
        let end = start + T::ENCODED_SIZE;
        assert!(end <= 32, "write out of bounds");

        if T::ENCODED_SIZE == 32 && offset == 0 {
            // Full slot overwrite - no need to read existing value
            let mut word = [0u8; 32];
            value.encode_into(&mut word);
            self.sstore(slot, B256::from(word))?;
            return Ok(());
        }

        // Partial update - preserve other bytes in slot
        let mut word = self.sload(slot)?;
        value.encode_into(&mut word.0[start..end]);
        self.sstore(slot, word)
    }
}

impl<S: StorageAPI> StorageOps for S {}
