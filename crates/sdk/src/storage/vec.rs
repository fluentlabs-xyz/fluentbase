use crate::{
    keccak256,
    storage::{
        primitive::StoragePrimitive, PackableCodec, StorageDescriptor, StorageLayout, StorageOps,
    },
    StorageAPI, B256, U256,
};
use core::marker::PhantomData;

/// Dynamic vector in storage.
/// Length at base slot, elements at keccak256(base_slot).
#[derive(Debug, PartialEq, Eq)]
pub struct StorageVec<T> {
    base_slot: U256,
    _marker: PhantomData<T>,
}

// Manual Copy/Clone to avoid T: Copy bound
impl<T> Clone for StorageVec<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for StorageVec<T> {}

impl<T> StorageVec<T> {
    pub const fn new(base_slot: U256) -> Self {
        Self {
            base_slot,
            _marker: PhantomData,
        }
    }

    /// Storage slot where elements start (keccak256 of base slot).
    fn elements_base_slot(&self) -> U256 {
        let hash = keccak256(self.base_slot.to_be_bytes::<32>());
        U256::from_be_bytes(hash.0)
    }
}

impl<T> StorageDescriptor for StorageVec<T> {
    fn new(slot: U256, offset: u8) -> Self {
        debug_assert_eq!(offset, 0, "vectors always start at slot boundary");
        Self::new(slot)
    }
    fn slot(&self) -> U256 {
        self.base_slot
    }

    fn offset(&self) -> u8 {
        0
    }
}

impl<T: StorageLayout> StorageVec<T>
where
    T::Descriptor: StorageDescriptor,
{
    /// Get current length of vector.
    pub fn len<S: StorageAPI>(&self, sdk: &S) -> u64 {
        let word = sdk.sload(self.base_slot);
        let mut len_bytes = [0u8; 8];
        len_bytes.copy_from_slice(&word.0[24..32]);
        u64::from_be_bytes(len_bytes)
    }

    /// Check if vector is empty.
    pub fn is_empty<S: StorageAPI>(&self, sdk: &S) -> bool {
        self.len(sdk) == 0
    }

    /// Calculate storage location for element at index.
    fn element_location(&self, index: u64) -> (U256, u8) {
        let elements_base = self.elements_base_slot();

        if T::SLOTS == 0 {
            // Packable elements
            let elements_per_slot = 32 / T::BYTES;
            let slot_index = index / elements_per_slot as u64;
            let position_in_slot = index % elements_per_slot as u64;

            // Pack from right to left (Solidity convention)
            let offset = (32 - (position_in_slot + 1) * T::BYTES as u64) as u8;

            (elements_base + U256::from(slot_index), offset)
        } else {
            // Non-packable elements
            (elements_base + U256::from(index * T::SLOTS as u64), 0)
        }
    }

    /// Access element at index (no bounds check).
    pub fn at(&self, index: u64) -> T::Accessor {
        let (slot, offset) = self.element_location(index);
        T::access(T::Descriptor::new(slot, offset))
    }

    /// Grow vector by one and return accessor to new element.
    pub fn grow<S: StorageAPI>(&self, sdk: &mut S) -> T::Accessor {
        let current_len = self.len(sdk);

        // Update length
        let new_len = current_len + 1;
        let mut len_bytes = [0u8; 32];
        len_bytes[24..32].copy_from_slice(&new_len.to_be_bytes());
        sdk.sstore(self.base_slot, B256::from(len_bytes));

        // Return accessor to new element
        self.at(current_len)
    }

    /// Shrink vector by one and return accessor to the removed element.
    /// The accessor remains valid until the slot is reused.
    pub fn shrink<S: StorageAPI>(&self, sdk: &mut S) -> Option<T::Accessor> {
        let current_len = self.len(sdk);
        if current_len == 0 {
            return None;
        }

        let index = current_len - 1;

        // Update length first
        let mut len_bytes = [0u8; 32];
        len_bytes[24..32].copy_from_slice(&index.to_be_bytes());
        sdk.sstore(self.base_slot, B256::from(len_bytes));

        // Return accessor to removed element (still in storage)
        Some(self.at(index))
    }
    /// Clear vector (sets length to 0).
    pub fn clear<S: StorageAPI>(&self, sdk: &mut S) {
        sdk.sstore(self.base_slot, B256::ZERO);
    }
}

/// Specialized API for vectors of primitive types.
impl<T: PackableCodec> StorageVec<StoragePrimitive<T>> {
    /// Push primitive value directly.
    pub fn push<S: StorageAPI>(&self, sdk: &mut S, value: T) {
        self.grow(sdk).set(sdk, value);
    }

    /// Remove and return last value.
    pub fn pop<S: StorageAPI>(&self, sdk: &mut S) -> Option<T> {
        self.shrink(sdk).map(|accessor| accessor.get(sdk))
    }
}

impl<T: StorageLayout> StorageLayout for StorageVec<T>
where
    T::Descriptor: StorageDescriptor,
{
    type Descriptor = Self;
    type Accessor = Self;

    const BYTES: usize = 32; // Only length stored inline
    const SLOTS: usize = 1; // One slot for length

    fn access(descriptor: Self::Descriptor) -> Self::Accessor {
        descriptor
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{
        mock::MockStorage,
        primitive::{StorageU256, StorageU64},
    };

    #[test]
    fn test_vec_primitive_api() {
        // Critical: test specialized push/pop for primitives
        let mut sdk = MockStorage::new();
        let vec = StorageVec::<StorageU256>::new(U256::from(100));

        // Push values
        vec.push(&mut sdk, U256::from(111));
        vec.push(&mut sdk, U256::from(222));
        vec.push(&mut sdk, U256::from(333));

        assert_eq!(vec.len(&sdk), 3);

        // Pop values
        assert_eq!(vec.pop(&mut sdk), Some(U256::from(333)));
        assert_eq!(vec.pop(&mut sdk), Some(U256::from(222)));
        assert_eq!(vec.len(&sdk), 1);

        // Access remaining
        assert_eq!(vec.at(0).get(&sdk), U256::from(111));

        // Pop from single element
        assert_eq!(vec.pop(&mut sdk), Some(U256::from(111)));
        assert_eq!(vec.pop(&mut sdk), None); // Empty
    }

    #[test]
    fn test_vec_packing() {
        // Critical: verify elements pack correctly (right to left)
        let mut sdk = MockStorage::new();
        let vec = StorageVec::<StorageU64>::new(U256::from(200));

        // Push 5 u64 values - should use 2 slots
        vec.push(&mut sdk, 0x1111111111111111u64);
        vec.push(&mut sdk, 0x2222222222222222u64);
        vec.push(&mut sdk, 0x3333333333333333u64);
        vec.push(&mut sdk, 0x4444444444444444u64);
        vec.push(&mut sdk, 0x5555555555555555u64);

        let elements_base = {
            let hash = keccak256(U256::from(200).to_be_bytes::<32>());
            U256::from_be_bytes(hash.0)
        };

        // First 4 packed in slot 0
        assert_eq!(
            sdk.get_slot_hex(elements_base),
            "4444444444444444333333333333333322222222222222221111111111111111"
        );

        // Fifth in slot 1
        assert_eq!(
            sdk.get_slot_hex(elements_base + U256::from(1)),
            "0000000000000000000000000000000000000000000000005555555555555555"
        );
    }

    #[test]
    fn test_vec_complex_types() {
        // Critical: verify grow/shrink for non-primitive types
        let mut sdk = MockStorage::new();
        let vec = StorageVec::<StorageVec<StorageU256>>::new(U256::from(300));

        // Grow and initialize nested vectors
        let inner1 = vec.grow(&mut sdk);
        inner1.push(&mut sdk, U256::from(10));
        inner1.push(&mut sdk, U256::from(20));

        let inner2 = vec.grow(&mut sdk);
        inner2.push(&mut sdk, U256::from(30));

        assert_eq!(vec.len(&sdk), 2);
        assert_eq!(vec.at(0).len(&sdk), 2);
        assert_eq!(vec.at(1).len(&sdk), 1);

        // Shrink returns accessor to removed element
        let removed = vec.shrink(&mut sdk).unwrap();
        assert_eq!(removed.at(0).get(&sdk), U256::from(30)); // Can still read
        assert_eq!(vec.len(&sdk), 1); // But length updated
    }
}
