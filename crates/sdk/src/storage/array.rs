use crate::storage::{StorageDescriptor, StorageLayout, U256};
use core::marker::PhantomData;

/// Fixed-size array in storage.
/// Arrays always start at slot boundaries.
#[derive(Debug, PartialEq, Eq)]
pub struct StorageArray<T, const N: usize> {
    base_slot: U256,
    _marker: PhantomData<T>,
}

// Manually implement Copy/Clone to avoid unnecessary T: Copy bound.
// StorageArray is just a descriptor (slot + phantom marker), not actual data.
// PhantomData<T> is always Copy regardless of T, so the descriptor can be Copy
// even when T represents non-Copy types like Vec or String.
impl<T, const N: usize> Clone for StorageArray<T, N> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T, const N: usize> Copy for StorageArray<T, N> {}

impl<T, const N: usize> StorageArray<T, N> {
    pub const fn new(base_slot: U256) -> Self {
        Self {
            base_slot,
            _marker: PhantomData,
        }
    }

    pub const fn base_slot(&self) -> U256 {
        self.base_slot
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

impl<T: StorageLayout, const N: usize> StorageArray<T, N>
where
    T::Descriptor: StorageDescriptor,
{
    /// Calculate how many elements fit in one slot (for packable types).
    #[inline]
    fn elements_per_slot() -> usize {
        if T::SLOTS == 0 {
            32 / T::BYTES
        } else {
            1
        }
    }

    /// Calculate storage location for element at index.
    fn element_location(&self, index: usize) -> (U256, u8) {
        if T::SLOTS == 0 {
            // Packable: multiple elements per slot
            let elements_per_slot = Self::elements_per_slot();
            let slot_index = index / elements_per_slot;
            let position_in_slot = index % elements_per_slot;

            // Pack from right to left (Solidity convention)
            let offset = (32 - (position_in_slot + 1) * T::BYTES) as u8;

            (self.base_slot + U256::from(slot_index), offset)
        } else {
            // Non-packable: each element uses T::SLOTS slots
            (self.base_slot + U256::from(index * T::SLOTS), 0)
        }
    }

    /// Access element at index.
    pub fn at(&self, index: usize) -> T::Accessor {
        assert!(index < N, "array index out of bounds");
        let (slot, offset) = self.element_location(index);
        T::access(T::Descriptor::new(slot, offset))
    }
}
impl<T: StorageLayout, const N: usize> StorageLayout for StorageArray<T, N> {
    type Descriptor = Self;
    type Accessor = Self;

    const BYTES: usize = if T::SLOTS == 0 {
        T::BYTES * N
    } else {
        T::SLOTS * N * 32
    };

    const SLOTS: usize = if T::SLOTS == 0 {
        (T::BYTES * N + 31) / 32
    } else {
        T::SLOTS * N
    };

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
    fn test_packing_correctness() {
        // Critical: verify array packing follows Solidity convention (right to left)
        let mut sdk = MockStorage::new();
        let array = StorageArray::<StorageU64, 5>::new(U256::from(10));

        // Set 5 u64 values (should use 2 slots: 4 in first, 1 in second)
        array.at(0).set(&mut sdk, 0x1111111111111111u64);
        array.at(1).set(&mut sdk, 0x2222222222222222u64);
        array.at(2).set(&mut sdk, 0x3333333333333333u64);
        array.at(3).set(&mut sdk, 0x4444444444444444u64);
        array.at(4).set(&mut sdk, 0x5555555555555555u64);

        // Verify packing: elements 0-3 in slot 10, element 4 in slot 11
        assert_eq!(
            sdk.get_slot_hex(U256::from(10)),
            "4444444444444444333333333333333322222222222222221111111111111111"
        );
        assert_eq!(
            sdk.get_slot_hex(U256::from(11)),
            "0000000000000000000000000000000000000000000000005555555555555555"
        );
    }

    #[test]
    fn test_nested_arrays() {
        // Critical: verify nested arrays calculate slots correctly
        let mut sdk = MockStorage::new();
        let array = StorageArray::<StorageArray<StorageU256, 2>, 3>::new(U256::from(50));

        // Should use 6 slots total
        assert_eq!(StorageArray::<StorageArray<StorageU256, 2>, 3>::SLOTS, 6);

        // Set and verify nested values
        array.at(0).at(0).set(&mut sdk, U256::from(100));
        array.at(2).at(1).set(&mut sdk, U256::from(301));

        assert_eq!(sdk.get_slot(U256::from(50)), U256::from(100));
        assert_eq!(sdk.get_slot(U256::from(55)), U256::from(301));
    }

    #[test]
    #[should_panic(expected = "array index out of bounds")]
    fn test_bounds_check() {
        let array = StorageArray::<StorageU256, 3>::new(U256::from(0));
        array.at(3); // Should panic
    }
}
