use crate::{
    keccak256,
    storage::{StorageDescriptor, StorageLayout, StorageOps, VecAccess},
    B256, U256,
};
use core::marker::PhantomData;
use fluentbase_types::StorageAPI;

// --- 1. Vec Descriptor ---

/// A descriptor for a dynamic vector in storage.
/// Follows Solidity's dynamic array storage layout:
/// - Length stored at base slot
/// - Elements start at keccak256(base_slot)
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct StorageVec<T> {
    base_slot: U256,
    _marker: PhantomData<T>,
}

impl<T> StorageVec<T> {
    /// Creates a new vector descriptor at the given base slot.
    pub const fn new(base_slot: U256) -> Self {
        Self {
            base_slot,
            _marker: PhantomData,
        }
    }

    /// Computes the storage slot for vector elements.
    /// Elements are stored starting at keccak256(base_slot).
    fn elements_base_slot(&self) -> U256 {
        // In Solidity, dynamic array elements start at keccak256(p)
        // where p is the base slot padded to 32 bytes
        let hash = keccak256(self.base_slot.to_be_bytes::<32>());
        U256::from_be_bytes(hash.0)
    }

    /// Calculates the slot and offset for an element at the given index.
    fn element_location(&self, index: u64) -> (U256, u8)
    where
        T: StorageLayout,
    {
        let elements_base = self.elements_base_slot();

        if T::REQUIRED_SLOTS == 0 {
            // Primitive types that can potentially be packed
            if T::ENCODED_SIZE < 32 {
                // Packable primitive - multiple elements per slot
                let elements_per_slot = 32 / T::ENCODED_SIZE;
                let slot_index = index / elements_per_slot as u64;
                let position_in_slot = index % elements_per_slot as u64;

                // Pack from left to right for arrays (high bytes to low bytes)
                // First element at offset 0, second at offset T::ENCODED_SIZE, etc.
                let offset = (32 - (position_in_slot + 1) * T::ENCODED_SIZE as u64) as u8;
                let slot = elements_base + U256::from(slot_index);

                (slot, offset)
            } else {
                // Full-width primitive (32 bytes) - one per slot
                let slot = elements_base + U256::from(index);
                (slot, 0)
            }
        } else {
            // Complex types - use REQUIRED_SLOTS slots per element
            let slot = elements_base + U256::from(index * T::REQUIRED_SLOTS as u64);
            (slot, 0)
        }
    }
}

impl<T> StorageDescriptor for StorageVec<T> {
    fn new(slot: U256, offset: u8) -> Self {
        debug_assert_eq!(offset, 0, "vectors always start at slot boundary");
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

// --- 2. VecAccess Implementation ---

impl<T: StorageLayout> VecAccess<T> for StorageVec<T>
where
    T::Descriptor: StorageDescriptor,
{
    fn len<S: StorageAPI>(&self, sdk: &S) -> u64 {
        // Length is stored at the base slot
        let length_word = sdk.sload(self.base_slot);
        // Convert from B256 to u64 (taking the least significant 8 bytes)
        let bytes = length_word.as_slice();
        let mut len_bytes = [0u8; 8];
        len_bytes.copy_from_slice(&bytes[24..32]);
        u64::from_be_bytes(len_bytes)
    }

    fn is_empty<S: StorageAPI>(&self, sdk: &S) -> bool {
        self.len(sdk) == 0
    }

    fn at(&self, index: u64) -> T::Accessor {
        let (slot, offset) = self.element_location(index);
        let element_descriptor = T::Descriptor::new(slot, offset);
        T::access(element_descriptor)
    }

    fn push<S: StorageAPI>(&self, sdk: &mut S) -> T::Accessor {
        // Get current length
        let current_len = self.len(sdk);

        // Update length (increment by 1)
        let new_len = current_len + 1;
        let mut len_bytes = [0u8; 32];
        len_bytes[24..32].copy_from_slice(&new_len.to_be_bytes());
        sdk.sstore(self.base_slot, B256::from(len_bytes));

        // Return accessor to the new element (at index = old length)
        let (slot, offset) = self.element_location(current_len);
        let element_descriptor = T::Descriptor::new(slot, offset);
        T::access(element_descriptor)
    }

    fn pop<S: StorageAPI>(&self, sdk: &mut S) {
        let current_len = self.len(sdk);
        if current_len == 0 {
            return; // Nothing to pop
        }

        // Update length (decrement by 1)
        let new_len = current_len - 1;
        let mut len_bytes = [0u8; 32];
        len_bytes[24..32].copy_from_slice(&new_len.to_be_bytes());
        sdk.sstore(self.base_slot, B256::from(len_bytes));

        // Note: We don't clear the storage slot for gas optimization
        // This matches Solidity's behavior
    }

    fn clear<S: StorageAPI>(&self, sdk: &mut S) {
        // Set the length to 0
        sdk.sstore(self.base_slot, B256::ZERO);
    }
}

// --- 3. StorageLayout Implementation ---

impl<T: StorageLayout> StorageLayout for StorageVec<T>
where
    T::Descriptor: StorageDescriptor,
{
    type Descriptor = StorageVec<T>;
    type Accessor = Self;

    // Dynamic vectors only need 1 slot for the length
    // Elements are stored separately at keccak256(base_slot)
    const REQUIRED_SLOTS: usize = 1;

    const ENCODED_SIZE: usize = 32; // Only the length is stored inline

    fn access(descriptor: Self::Descriptor) -> Self::Accessor {
        descriptor
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        keccak256,
        storage::{mock::MockStorage, primitive::StoragePrimitive, PrimitiveAccess},
    };

    #[test]
    fn test_vec_push_and_access() {
        let mut sdk = MockStorage::new();
        let vec = StorageVec::<StoragePrimitive<U256>>::new(U256::from(100));

        // Initially empty
        assert_eq!(vec.len(&sdk), 0);
        assert!(vec.is_empty(&sdk));

        // Push the first element
        vec.push(&mut sdk).set(&mut sdk, U256::from(111));
        assert_eq!(vec.len(&sdk), 1);

        // Push the second element
        vec.push(&mut sdk).set(&mut sdk, U256::from(222));
        assert_eq!(vec.len(&sdk), 2);

        // Access elements - need to check bounds first
        let current_len = vec.len(&sdk);
        assert!(
            0 < current_len,
            "vector index out of bounds: index 0 >= length {current_len}"
        );
        assert!(
            1 < current_len,
            "vector index out of bounds: index 1 >= length {current_len}"
        );

        assert_eq!(vec.at(0).get(&sdk), U256::from(111));
        assert_eq!(vec.at(1).get(&sdk), U256::from(222));
    }

    #[test]
    fn test_vec_pop() {
        let mut sdk = MockStorage::new();
        let vec = StorageVec::<StoragePrimitive<U256>>::new(U256::from(200));

        // Push elements
        vec.push(&mut sdk).set(&mut sdk, U256::from(100));
        vec.push(&mut sdk).set(&mut sdk, U256::from(200));
        vec.push(&mut sdk).set(&mut sdk, U256::from(300));
        assert_eq!(vec.len(&sdk), 3);

        // Pop one element
        vec.pop(&mut sdk);
        assert_eq!(vec.len(&sdk), 2);

        // Remaining elements still accessible
        assert_eq!(vec.at(0).get(&sdk), U256::from(100));
        assert_eq!(vec.at(1).get(&sdk), U256::from(200));
    }

    #[test]
    fn test_vec_packed_elements() {
        let mut sdk = MockStorage::new();
        let vec = StorageVec::<StoragePrimitive<u64>>::new(U256::from(300));

        // Push multiple u64 values (should pack)
        vec.push(&mut sdk).set(&mut sdk, 0x1111111111111111u64);
        vec.push(&mut sdk).set(&mut sdk, 0x2222222222222222u64);
        vec.push(&mut sdk).set(&mut sdk, 0x3333333333333333u64);
        vec.push(&mut sdk).set(&mut sdk, 0x4444444444444444u64);

        assert_eq!(vec.len(&sdk), 4);

        // Verify packing by checking the storage
        // Elements should be packed in the first slot after keccak(base_slot)
        let elements_base = {
            let hash = keccak256(U256::from(300).to_be_bytes::<32>());
            U256::from_be_bytes(hash.0)
        };

        // All 4 u64 values should be packed in one slot
        let slot_content = sdk.get_slot_hex(elements_base);
        assert_eq!(
            slot_content,
            "4444444444444444333333333333333322222222222222221111111111111111"
        );
    }

    #[test]
    fn test_vec_packed_elements2() {
        let mut sdk = MockStorage::new();
        let vec = StorageVec::<StoragePrimitive<u64>>::new(U256::from(300));

        vec.push(&mut sdk).set(&mut sdk, 0x1111111111111111u64);
        vec.push(&mut sdk).set(&mut sdk, 0x2222222222222222u64);
        vec.push(&mut sdk).set(&mut sdk, 0x3333333333333333u64);
        vec.push(&mut sdk).set(&mut sdk, 0x4444444444444444u64);

        let elements_base = {
            let hash = keccak256(U256::from(300).to_be_bytes::<32>());
            U256::from_be_bytes(hash.0)
        };

        // Добавьте отладочный вывод
        println!("Elements base slot: {elements_base:?}");
        let raw_value = sdk.get_slot(elements_base);
        println!("Raw U256 value: {raw_value:?}");
        println!("As hex: {}", sdk.get_slot_hex(elements_base));

        // Проверим offset для каждого элемента
        for i in 0..4 {
            let (slot, offset) = vec.element_location(i);
            println!("Element {i}: slot={slot:?}, offset={offset}");
        }
    }

    #[test]
    fn test_vec_clear() {
        let mut sdk = MockStorage::new();
        let vec = StorageVec::<StoragePrimitive<U256>>::new(U256::from(400));

        // Push elements
        vec.push(&mut sdk).set(&mut sdk, U256::from(1));
        vec.push(&mut sdk).set(&mut sdk, U256::from(2));
        assert_eq!(vec.len(&sdk), 2);

        // Clear
        vec.clear(&mut sdk);
        assert_eq!(vec.len(&sdk), 0);
        assert!(vec.is_empty(&sdk));
    }

    #[test]
    fn test_vec_bounds_check() {
        let mut sdk = MockStorage::new();
        let vec = StorageVec::<StoragePrimitive<U256>>::new(U256::from(500));

        vec.push(&mut sdk).set(&mut sdk, U256::from(100));

        // Test that we can access a valid index
        assert_eq!(vec.at(0).get(&sdk), U256::from(100));

        // Note: at() method itself doesn't perform bound's checking since it doesn't have access to SDK
        // Bounds checking would need to be done by the caller or in the accessor methods
    }

    #[test]
    fn test_nested_vec() {
        let mut sdk = MockStorage::new();
        let vec = StorageVec::<StorageVec<StoragePrimitive<U256>>>::new(U256::from(600));

        // Push a nested vector
        let inner_vec = vec.push(&mut sdk);

        // Push elements to the inner vector
        inner_vec.push(&mut sdk).set(&mut sdk, U256::from(10));
        inner_vec.push(&mut sdk).set(&mut sdk, U256::from(20));

        // Verify
        assert_eq!(vec.len(&sdk), 1);
        assert_eq!(vec.at(0).len(&sdk), 2);
        assert_eq!(vec.at(0).at(0).get(&sdk), U256::from(10));
        assert_eq!(vec.at(0).at(1).get(&sdk), U256::from(20));
    }
}
