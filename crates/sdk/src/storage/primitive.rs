use crate::storage::{
    PrimitiveAccess, PrimitiveCodec, StorageDescriptor, StorageLayout, StorageOps,
};
use core::marker::PhantomData;
use fluentbase_types::{Address, StorageAPI, U256};

// --- 1. Descriptor ---

/// A lightweight, copy-able descriptor that defines the storage location
/// of a single, packable value.
#[derive(Default, Copy, Clone, Debug, PartialEq, Eq)]
pub struct StoragePrimitive<T> {
    slot: U256,
    offset: u8,
    _marker: PhantomData<T>,
}

impl<T> StoragePrimitive<T> {
    /// Creates a new descriptor for a primitive value at a specific location.
    pub const fn new(slot: U256, offset: u8) -> Self {
        Self {
            slot,
            offset,
            _marker: PhantomData,
        }
    }
}

impl<T> StorageDescriptor for StoragePrimitive<T> {
    fn new(slot: U256, offset: u8) -> Self {
        Self::new(slot, offset)
    }

    fn slot(&self) -> U256 {
        self.slot
    }

    fn offset(&self) -> u8 {
        self.offset
    }
}

// --- 2. Accessor ---

/// A lightweight accessor that holds only the storage location descriptor.
/// SDK is passed to methods that need storage access.
pub struct PrimitiveAccessor<T: PrimitiveCodec> {
    descriptor: StoragePrimitive<T>,
}

impl<T: PrimitiveCodec> PrimitiveAccessor<T> {
    /// Creates a new accessor.
    pub(crate) fn new(descriptor: StoragePrimitive<T>) -> Self {
        Self { descriptor }
    }
}

// --- 3. API Implementation (impl PrimitiveAccess for PrimitiveAccessor) ---

impl<T: PrimitiveCodec> PrimitiveAccess<T> for PrimitiveAccessor<T> {
    /// Reads the value from its storage slot, decodes it, and returns it.
    fn get<S: StorageAPI>(&self, sdk: &S) -> T {
        sdk.read_at(self.descriptor.slot, self.descriptor.offset)
    }

    /// Encodes and writes a new value to its storage slot.
    fn set<S: StorageAPI>(&self, sdk: &mut S, value: T) {
        sdk.write_at(self.descriptor.slot, self.descriptor.offset, &value);
    }
}

// --- 4. Main Trait Implementation (impl StorageLayout for Primitive) ---

impl<T: PrimitiveCodec + Copy> StorageLayout for StoragePrimitive<T> {
    /// The descriptor for a `Primitive` is the struct itself.
    type Descriptor = Self;

    /// The accessor for a `Primitive` is the `PrimitiveAccessor`.
    type Accessor = PrimitiveAccessor<T>;

    /// A single primitive value, even if small, reserves at least one full slot
    /// in the storage layout calculation to avoid complex packing across fields.
    /// Packing happens *within* the slot, managed by the `offset`.
    const REQUIRED_SLOTS: usize = if T::ENCODED_SIZE == 32 { 1 } else { 0 };

    const ENCODED_SIZE: usize = T::ENCODED_SIZE;

    /// The entry point to interacting with the primitive value.
    fn access(descriptor: Self::Descriptor) -> Self::Accessor {
        PrimitiveAccessor::new(descriptor)
    }
}

// --- 5. StorageCodec Implementations for Core Primitives ---

impl PrimitiveCodec for U256 {
    const ENCODED_SIZE: usize = 32;

    fn encode_into(&self, target: &mut [u8]) {
        target.copy_from_slice(&self.to_be_bytes::<32>());
    }

    fn decode(bytes: &[u8]) -> Self {
        U256::from_be_bytes::<32>(bytes.try_into().unwrap())
    }
}

impl PrimitiveCodec for Address {
    const ENCODED_SIZE: usize = 20;

    fn encode_into(&self, target: &mut [u8]) {
        target.copy_from_slice(self.as_slice());
    }

    fn decode(bytes: &[u8]) -> Self {
        Address::from_slice(bytes)
    }
}

impl PrimitiveCodec for bool {
    const ENCODED_SIZE: usize = 1;

    fn encode_into(&self, target: &mut [u8]) {
        target[0] = *self as u8;
    }

    fn decode(bytes: &[u8]) -> Self {
        bytes[0] != 0
    }
}

impl PrimitiveCodec for u64 {
    const ENCODED_SIZE: usize = 8;

    fn encode_into(&self, target: &mut [u8]) {
        target.copy_from_slice(&self.to_be_bytes());
    }

    fn decode(bytes: &[u8]) -> Self {
        u64::from_be_bytes(bytes.try_into().unwrap())
    }
}

impl PrimitiveCodec for u32 {
    const ENCODED_SIZE: usize = 4;

    fn encode_into(&self, target: &mut [u8]) {
        target.copy_from_slice(&self.to_be_bytes());
    }

    fn decode(bytes: &[u8]) -> Self {
        u32::from_be_bytes(bytes.try_into().unwrap())
    }
}

impl PrimitiveCodec for u16 {
    const ENCODED_SIZE: usize = 2;

    fn encode_into(&self, target: &mut [u8]) {
        target.copy_from_slice(&self.to_be_bytes());
    }

    fn decode(bytes: &[u8]) -> Self {
        u16::from_be_bytes(bytes.try_into().unwrap())
    }
}

impl PrimitiveCodec for u8 {
    const ENCODED_SIZE: usize = 1;

    fn encode_into(&self, target: &mut [u8]) {
        target[0] = *self;
    }

    fn decode(bytes: &[u8]) -> Self {
        bytes[0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::mock::MockStorage;

    #[test]
    fn read_write_u256() {
        let mut sdk = MockStorage::new();
        let counter = StoragePrimitive::<U256>::new(U256::from(1), 0);
        let accessor = <StoragePrimitive<U256> as StorageLayout>::access(counter);

        let value = accessor.get(&sdk);
        assert_eq!(value, U256::ZERO);

        accessor.set(&mut sdk, U256::from(1));
        let value = accessor.get(&sdk);
        assert_eq!(value, U256::from(1));
    }

    #[test]
    fn set_and_get_work_correctly() {
        let mut sdk = MockStorage::new();
        let counter = StoragePrimitive::<U256>::new(U256::from(2), 0);
        let new_value = U256::from(12345);
        let accessor = <StoragePrimitive<U256> as StorageLayout>::access(counter);

        // Act: Set the value
        accessor.set(&mut sdk, new_value);

        // Assert: Read it back
        let read_value = accessor.get(&sdk);
        assert_eq!(read_value, new_value);

        // Assert: Check raw slot content
        assert_eq!(sdk.get_slot(U256::from(2)), new_value);
    }

    #[test]
    fn packed_values_do_not_interfere() {
        let mut sdk = MockStorage::new();
        let slot = U256::from(3);

        // Layout: [ u64 (offset 23) | bool (offset 31) ]
        let flag_accessor =
            <StoragePrimitive<bool> as StorageLayout>::access(StoragePrimitive::<bool>::new(
                slot, 31,
            ));
        let timestamp_accessor = <StoragePrimitive<u64> as StorageLayout>::access(
            StoragePrimitive::<u64>::new(slot, 23),
        );

        // Act
        flag_accessor.set(&mut sdk, true);
        timestamp_accessor.set(&mut sdk, 0xDEADBEEFCAFEBABE);

        // Assert
        assert!(flag_accessor.get(&sdk));
        assert_eq!(timestamp_accessor.get(&sdk), 0xDEADBEEFCAFEBABE);

        // Assert raw slot content via snapshot
        let expected_hex = "0000000000000000000000000000000000000000000000deadbeefcafebabe01";
        assert_eq!(sdk.get_slot_hex(U256::from(3)), expected_hex);
    }

    #[test]
    fn set_optimizes_for_full_width_types() {
        let mut sdk = MockStorage::new();
        // Initialize slot with junk data
        sdk.init_slot(
            U256::from(3),
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        );

        let accessor = <StoragePrimitive<U256> as StorageLayout>::access(
            StoragePrimitive::<U256>::new(U256::from(4), 0),
        );
        let new_value = U256::from(1);

        // Act: This `set` should NOT read the old value but overwrite it completely.
        accessor.set(&mut sdk, new_value);

        // Assert
        assert_eq!(
            sdk.get_slot_hex(U256::from(4)),
            "0000000000000000000000000000000000000000000000000000000000000001"
        );
    }

    #[test]
    fn different_primitive_types() {
        let mut sdk = MockStorage::new();
        let slot = U256::from(5);

        // Test different primitive types
        let bool_accessor =
            <StoragePrimitive<bool> as StorageLayout>::access(StoragePrimitive::<bool>::new(
                slot, 31,
            ));
        let u8_accessor =
            <StoragePrimitive<u8> as StorageLayout>::access(StoragePrimitive::<u8>::new(slot, 30));
        let u16_accessor = <StoragePrimitive<u16> as StorageLayout>::access(
            StoragePrimitive::<u16>::new(slot, 28),
        );
        let u32_accessor = <StoragePrimitive<u32> as StorageLayout>::access(
            StoragePrimitive::<u32>::new(slot, 24),
        );

        // Set values
        bool_accessor.set(&mut sdk, true);
        u8_accessor.set(&mut sdk, 0xFF);
        u16_accessor.set(&mut sdk, 0xABCD);
        u32_accessor.set(&mut sdk, 0x12345678);

        // Verify values
        assert!(bool_accessor.get(&sdk));
        assert_eq!(u8_accessor.get(&sdk), 0xFF);
        assert_eq!(u16_accessor.get(&sdk), 0xABCD);
        assert_eq!(u32_accessor.get(&sdk), 0x12345678);
    }
}
