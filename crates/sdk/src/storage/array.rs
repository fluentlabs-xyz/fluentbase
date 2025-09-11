use crate::{
    storage::{ArrayAccess, StorageDescriptor, StorageLayout},
    U256,
};
use core::marker::PhantomData;

// --- 1. Array Descriptor ---

/// A descriptor for a fixed-size array in storage.
/// Arrays always start at slot boundaries (offset = 0).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct StorageArray<T, const N: usize> {
    base_slot: U256,
    _marker: PhantomData<T>,
}

impl<T, const N: usize> StorageArray<T, N> {
    /// Creates a new array descriptor at the given base slot.
    pub const fn new(base_slot: U256) -> Self {
        Self {
            base_slot,
            _marker: PhantomData,
        }
    }
}

impl<T, const N: usize> StorageDescriptor for StorageArray<T, N> {
    fn new(slot: U256, offset: u8) -> Self {
        debug_assert_eq!(offset, 0, "arrays always start at slot boundary");
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

// --- 2. ArrayAccess Implementation ---

impl<T: StorageLayout, const N: usize> ArrayAccess<T, N> for StorageArray<T, N>
where
    T::Descriptor: StorageDescriptor,
{
    fn at(&self, index: usize) -> T::Accessor {
        assert!(index < N, "array index out of bounds");
        let (slot, offset) = if T::ENCODED_SIZE <= 32 && T::REQUIRED_SLOTS == 0 {
            let element_size = T::ENCODED_SIZE;
            let elements_per_slot = 32 / element_size;

            let slot_index = index / elements_per_slot;
            let position_in_slot = index % elements_per_slot;

            // Solidity packs from right to left
            // the First element goes to the rightmost position
            // offset = start position from left edge
            // For the element at position 0 (rightmost): offset = 32 - element_size
            // For the element at position 1: offset = 32-2*element_size
            let offset = (32 - (position_in_slot + 1) * element_size) as u8;

            (self.base_slot + U256::from(slot_index), offset)
        } else {
            // Non-packable types
            let slots = if T::REQUIRED_SLOTS == 0 {
                1
            } else {
                T::REQUIRED_SLOTS
            };
            (self.base_slot + U256::from(index * slots), 0)
        };

        T::access(T::Descriptor::new(slot, offset))
    }
}

// --- 3. StorageLayout Implementation ---

impl<T: StorageLayout, const N: usize> StorageLayout for StorageArray<T, N>
where
    T::Descriptor: StorageDescriptor,
{
    type Descriptor = StorageArray<T, N>;

    // Array acts as its own accessor - no separate accessor type needed
    type Accessor = Self;

    const REQUIRED_SLOTS: usize = {
        if T::REQUIRED_SLOTS == 0 {
            let elements_per_slot = 32 / T::ENCODED_SIZE;
            N.div_ceil(elements_per_slot)
        } else {
            N * T::REQUIRED_SLOTS
        }
    };

    const ENCODED_SIZE: usize = Self::REQUIRED_SLOTS * 32;

    fn access(descriptor: Self::Descriptor) -> Self::Accessor {
        // Return the descriptor itself as it acts as the accessor
        descriptor
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::mock::MockStorage;
    use crate::storage::primitive::StoragePrimitive;
    use crate::storage::PrimitiveAccess;

    #[test]
    fn test_u256_array_no_packing() {
        let mut sdk = MockStorage::new();
        let array = StorageArray::<StoragePrimitive<U256>, 3>::new(U256::from(10));

        // Each U256 takes a full slot - no packing possible
        assert_eq!(
            <StorageArray<StoragePrimitive<U256>, 3> as StorageLayout>::REQUIRED_SLOTS,
            3
        );

        // Direct array access without intermediate accessor
        array.at(0).set(&mut sdk, U256::from(100));
        array.at(1).set(&mut sdk, U256::from(200));
        array.at(2).set(&mut sdk, U256::from(300));

        // Verify each value is in its own slot
        assert_eq!(sdk.get_slot(U256::from(10)), U256::from(100));
        assert_eq!(sdk.get_slot(U256::from(11)), U256::from(200));
        assert_eq!(sdk.get_slot(U256::from(12)), U256::from(300));
    }

    #[test]
    fn test_u64_array_packing() {
        let mut sdk = MockStorage::new();
        let array = StorageArray::<StoragePrimitive<u64>, 4>::new(U256::from(20));

        // 4 u64 values (8 bytes each) should pack into 1 slot (32 bytes)
        assert_eq!(
            <StorageArray<StoragePrimitive<u64>, 4> as StorageLayout>::REQUIRED_SLOTS,
            1
        );

        array.at(0).set(&mut sdk, 0x1111111111111111u64);
        array.at(1).set(&mut sdk, 0x2222222222222222u64);
        array.at(2).set(&mut sdk, 0x3333333333333333u64);
        array.at(3).set(&mut sdk, 0x4444444444444444u64);

        // All values packed in slot 20, from right to left
        let expected = "4444444444444444333333333333333322222222222222221111111111111111";
        assert_eq!(sdk.get_slot_hex(U256::from(20)), expected);
    }

    #[test]
    fn test_bool_array_packing() {
        let mut sdk = MockStorage::new();
        let array = StorageArray::<StoragePrimitive<bool>, 32>::new(U256::from(30));

        // 32 bools (1 byte each) should pack into 1 slot
        assert_eq!(
            <StorageArray<StoragePrimitive<bool>, 32> as StorageLayout>::REQUIRED_SLOTS,
            1
        );

        // Set some values
        array.at(0).set(&mut sdk, true);
        array.at(1).set(&mut sdk, false);
        array.at(2).set(&mut sdk, true);
        array.at(31).set(&mut sdk, true);

        // Read back
        assert!(array.at(0).get(&sdk));
        assert!(!array.at(1).get(&sdk));
        assert!(array.at(2).get(&sdk));
        assert!(array.at(31).get(&sdk));
    }

    #[test]
    fn test_nested_array() {
        let mut sdk = MockStorage::new();
        // Array of arrays: 3 arrays, each containing 2 U256 values
        let array = StorageArray::<StorageArray<StoragePrimitive<U256>, 2>, 3>::new(U256::from(50));

        // Should take 6 slots total (3 arrays * 2 slots each)
        assert_eq!(
            <StorageArray<StorageArray<StoragePrimitive<U256>, 2>, 3> as StorageLayout>::REQUIRED_SLOTS,
            6
        );

        // Access nested elements
        array.at(0).at(0).set(&mut sdk, U256::from(100));
        array.at(0).at(1).set(&mut sdk, U256::from(101));
        array.at(1).at(0).set(&mut sdk, U256::from(200));
        array.at(1).at(1).set(&mut sdk, U256::from(201));
        array.at(2).at(0).set(&mut sdk, U256::from(300));
        array.at(2).at(1).set(&mut sdk, U256::from(301));

        // Verify storage layout - sequential slots for nested arrays
        assert_eq!(sdk.get_slot(U256::from(50)), U256::from(100)); // array[0][0]
        assert_eq!(sdk.get_slot(U256::from(51)), U256::from(101)); // array[0][1]
        assert_eq!(sdk.get_slot(U256::from(52)), U256::from(200)); // array[1][0]
        assert_eq!(sdk.get_slot(U256::from(53)), U256::from(201)); // array[1][1]
        assert_eq!(sdk.get_slot(U256::from(54)), U256::from(300)); // array[2][0]
        assert_eq!(sdk.get_slot(U256::from(55)), U256::from(301)); // array[2][1]
    }

    #[test]
    #[should_panic(expected = "array index out of bounds")]
    fn test_bounds_check() {
        let array = StorageArray::<StoragePrimitive<U256>, 3>::new(U256::from(60));

        // This should panic - index 3 is out of bounds for an array of size 3
        array.at(3);
    }

    #[test]
    fn test_packed_array_multiple_slots() {
        let mut sdk = MockStorage::new();
        let array = StorageArray::<StoragePrimitive<u64>, 5>::new(U256::from(70));

        // 5 u64 values need 2 slots (4 in the first slot, 1 in the second)
        assert_eq!(
            <StorageArray<StoragePrimitive<u64>, 5> as StorageLayout>::REQUIRED_SLOTS,
            2
        );

        array.at(0).set(&mut sdk, 0x1111111111111111u64);
        array.at(1).set(&mut sdk, 0x2222222222222222u64);
        array.at(2).set(&mut sdk, 0x3333333333333333u64);
        array.at(3).set(&mut sdk, 0x4444444444444444u64);
        array.at(4).set(&mut sdk, 0x5555555555555555u64);

        // First 4 packed in slot 70
        let expected_slot_70 = "4444444444444444333333333333333322222222222222221111111111111111";
        assert_eq!(sdk.get_slot_hex(U256::from(70)), expected_slot_70);

        // Fifth element in slot 71
        let expected_slot_71 = "0000000000000000000000000000000000000000000000005555555555555555";
        assert_eq!(sdk.get_slot_hex(U256::from(71)), expected_slot_71);
    }

    #[test]
    fn test_array_get_operations() {
        let mut sdk = MockStorage::new();
        let array = StorageArray::<StoragePrimitive<u32>, 8>::new(U256::from(80));

        // Set values first
        for i in 0..8 {
            array.at(i).set(&mut sdk, (i as u32 + 1) * 100);
        }

        // Test get operations
        for i in 0..8 {
            assert_eq!(array.at(i).get(&sdk), (i as u32 + 1) * 100);
        }
    }
}
