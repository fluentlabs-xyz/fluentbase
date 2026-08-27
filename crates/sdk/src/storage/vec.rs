use crate::{
    keccak256,
    storage::{
        primitive::StoragePrimitive, PackableCodec, StorageDescriptor, StorageLayout, StorageOps,
    },
    StorageAPI, B256, U256,
};
use core::marker::PhantomData;
use fluentbase_types::ExitCode;

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
        self.len_checked(sdk).unwrap()
    }

    pub fn len_checked<S: StorageAPI>(&self, sdk: &S) -> Result<u64, ExitCode> {
        let word = sdk.sload(self.base_slot)?;
        let mut len_bytes = [0u8; 8];
        len_bytes.copy_from_slice(&word.0[24..32]);
        Ok(u64::from_be_bytes(len_bytes))
    }

    /// Check if vector is empty.
    pub fn is_empty<S: StorageAPI>(&self, sdk: &S) -> bool {
        self.is_empty_checked(sdk).unwrap()
    }

    pub fn is_empty_checked<S: StorageAPI>(&self, sdk: &S) -> Result<bool, ExitCode> {
        Ok(self.len_checked(sdk)? == 0)
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
            // Non-packable elements. The multiplication is done in U256 so that a large index
            // cannot wrap a u64 and alias an earlier element (contracts build with
            // `overflow-checks = false`).
            (elements_base + U256::from(index) * U256::from(T::SLOTS), 0)
        }
    }

    /// Access element at index without checking it against the current length.
    ///
    /// Distinct indices always map to distinct storage locations, but indices past the
    /// length address slots the vector does not own yet. Prefer [`Self::get`].
    pub fn at(&self, index: u64) -> T::Accessor {
        let (slot, offset) = self.element_location(index);
        T::access(T::Descriptor::new(slot, offset))
    }

    /// Access element at index, returning `None` when it is out of bounds.
    pub fn get<S: StorageAPI>(&self, sdk: &S, index: u64) -> Option<T::Accessor> {
        self.get_checked(sdk, index).unwrap()
    }

    pub fn get_checked<S: StorageAPI>(
        &self,
        sdk: &S,
        index: u64,
    ) -> Result<Option<T::Accessor>, ExitCode> {
        if index >= self.len_checked(sdk)? {
            return Ok(None);
        }
        Ok(Some(self.at(index)))
    }

    /// Grow vector by one and return accessor to new element.
    pub fn grow<S: StorageAPI>(&self, sdk: &mut S) -> T::Accessor {
        self.grow_checked(sdk).unwrap()
    }

    pub fn grow_checked<S: StorageAPI>(&self, sdk: &mut S) -> Result<T::Accessor, ExitCode> {
        let current_len = self.len_checked(sdk)?;

        // Update length. Contracts build with `overflow-checks = false`, so the increment is
        // checked explicitly rather than relying on a debug-only panic.
        let new_len = current_len
            .checked_add(1)
            .ok_or(ExitCode::IntegerOverflow)?;
        let mut len_bytes = [0u8; 32];
        len_bytes[24..32].copy_from_slice(&new_len.to_be_bytes());
        sdk.sstore(self.base_slot, B256::from(len_bytes))?;

        // Return accessor to new element
        Ok(self.at(current_len))
    }

    /// Shrink vector by one and return accessor to the removed element.
    /// The accessor remains valid until the slot is reused.
    pub fn shrink<S: StorageAPI>(&self, sdk: &mut S) -> Option<T::Accessor> {
        self.shrink_checked(sdk).unwrap()
    }

    pub fn shrink_checked<S: StorageAPI>(
        &self,
        sdk: &mut S,
    ) -> Result<Option<T::Accessor>, ExitCode> {
        let current_len = self.len_checked(sdk)?;
        if current_len == 0 {
            return Ok(None);
        }

        let index = current_len - 1;

        // Update length first
        let mut len_bytes = [0u8; 32];
        len_bytes[24..32].copy_from_slice(&index.to_be_bytes());
        sdk.sstore(self.base_slot, B256::from(len_bytes))?;

        // Return accessor to removed element (still in storage)
        Ok(Some(self.at(index)))
    }

    /// Clear vector (sets length to 0).
    pub fn clear<S: StorageAPI>(&self, sdk: &mut S) {
        self.clear_checked(sdk).unwrap()
    }

    pub fn clear_checked<S: StorageAPI>(&self, sdk: &mut S) -> Result<(), ExitCode> {
        sdk.sstore(self.base_slot, B256::ZERO)
    }
}

/// Specialized API for vectors of primitive types.
impl<T: PackableCodec> StorageVec<StoragePrimitive<T>> {
    /// Push primitive value directly.
    pub fn push<S: StorageAPI>(&self, sdk: &mut S, value: T) {
        self.push_checked(sdk, value).unwrap()
    }

    pub fn push_checked<S: StorageAPI>(&self, sdk: &mut S, value: T) -> Result<(), ExitCode> {
        self.grow_checked(sdk)?.set_checked(sdk, value)
    }

    /// Remove and return last value.
    pub fn pop<S: StorageAPI>(&self, sdk: &mut S) -> Option<T> {
        self.pop_checked(sdk).unwrap()
    }

    pub fn pop_checked<S: StorageAPI>(&self, sdk: &mut S) -> Result<Option<T>, ExitCode> {
        self.shrink_checked(sdk)?
            .map(|accessor| accessor.get_checked(sdk))
            .transpose()
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
        array::StorageArray,
        mock::MockStorage,
        primitive::{StorageU256, StorageU64, StorageU8},
    };

    /// Indices spanning both the packing boundaries and the u64 values that used to wrap.
    const PROBE_INDICES: [u64; 14] = [
        0,
        1,
        2,
        3,
        4,
        7,
        31,
        32,
        33,
        1 << 32,
        (1 << 63) - 1,
        1 << 63,
        u64::MAX - 1,
        u64::MAX,
    ];

    /// Total order over element addresses: slots grow with the index, and elements packed
    /// inside one slot are laid out right to left (so a lower offset means a later element).
    fn address_key<T: StorageLayout>(vec: &StorageVec<T>, index: u64) -> (U256, u8)
    where
        T::Descriptor: StorageDescriptor,
    {
        let (slot, offset) = vec.element_location(index);
        (slot.wrapping_sub(vec.elements_base_slot()), 32 - offset)
    }

    fn assert_addresses_monotonic<T: StorageLayout>(vec: &StorageVec<T>)
    where
        T::Descriptor: StorageDescriptor,
    {
        for pair in PROBE_INDICES.windows(2) {
            let (lower, higher) = (pair[0], pair[1]);
            assert!(
                address_key(vec, lower) < address_key(vec, higher),
                "index {higher} does not address a later location than {lower} (SLOTS={}, BYTES={})",
                T::SLOTS,
                T::BYTES,
            );
        }
    }

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

    #[test]
    fn test_vec_large_index_does_not_alias_earlier_element() {
        // Elements of this vector reserve 2 slots each, so `index * SLOTS` used to wrap a
        // u64 back to 0 at index 2^63 and alias element 0.
        let vec = StorageVec::<StorageArray<StorageU256, 2>>::new(U256::from(400));
        let elements_base = vec.elements_base_slot();

        assert_eq!(vec.element_location(0), (elements_base, 0));

        let (slot, offset) = vec.element_location(1 << 63);
        assert_eq!(offset, 0);
        assert_eq!(
            slot.wrapping_sub(elements_base),
            U256::from(1u64 << 63) * U256::from(2)
        );
    }

    #[test]
    fn test_vec_addresses_are_monotonic_for_all_widths() {
        // Packed elements, one element per slot, and multi-slot elements.
        assert_addresses_monotonic(&StorageVec::<StorageU8>::new(U256::from(401)));
        assert_addresses_monotonic(&StorageVec::<StorageU64>::new(U256::from(402)));
        assert_addresses_monotonic(&StorageVec::<StorageU256>::new(U256::from(403)));
        assert_addresses_monotonic(&StorageVec::<StorageArray<StorageU256, 2>>::new(
            U256::from(404),
        ));
        assert_addresses_monotonic(&StorageVec::<StorageArray<StorageU256, 7>>::new(
            U256::from(405),
        ));
    }

    /// A storage backend whose reads always fail, mirroring the `MissingStorageSlot` a system
    /// runtime returns for a slot the executor did not preload.
    #[derive(Default)]
    struct UnreadableStorage;

    impl StorageAPI for UnreadableStorage {
        fn write_storage(&mut self, _slot: U256, _value: U256) -> crate::SyscallResult<()> {
            crate::SyscallResult::new((), 0, 0, ExitCode::Ok)
        }

        fn storage(&self, _slot: &U256) -> crate::SyscallResult<U256> {
            crate::SyscallResult::new(U256::ZERO, 0, 0, ExitCode::MissingStorageSlot)
        }
    }

    /// The `_checked` family must surface a failing read to the caller. It used to route through
    /// the panicking `len`, so an unreadable length slot aborted the frame instead of returning.
    #[test]
    fn test_checked_growth_propagates_storage_read_failure() {
        let vec = StorageVec::<StorageU256>::new(U256::from(600));
        let mut sdk = UnreadableStorage;

        assert_eq!(
            vec.grow_checked(&mut sdk).err(),
            Some(ExitCode::MissingStorageSlot)
        );
        assert_eq!(
            vec.push_checked(&mut sdk, U256::from(1)),
            Err(ExitCode::MissingStorageSlot)
        );
    }

    #[test]
    fn test_checked_shrink_propagates_storage_read_failure() {
        let vec = StorageVec::<StorageU256>::new(U256::from(601));
        let mut sdk = UnreadableStorage;

        assert_eq!(
            vec.shrink_checked(&mut sdk).err(),
            Some(ExitCode::MissingStorageSlot)
        );
        assert_eq!(
            vec.pop_checked(&mut sdk).err(),
            Some(ExitCode::MissingStorageSlot)
        );
    }

    /// A vector already at `u64::MAX` must not wrap its length back to zero and alias element
    /// zero, which is what the unchecked increment did with `overflow-checks = false`.
    #[test]
    fn test_grow_checked_rejects_length_overflow() {
        let mut sdk = MockStorage::new();
        let vec = StorageVec::<StorageU256>::new(U256::from(602));
        sdk.write_storage(U256::from(602), U256::from(u64::MAX));

        assert_eq!(
            vec.grow_checked(&mut sdk).err(),
            Some(ExitCode::IntegerOverflow)
        );
    }

    #[test]
    fn test_vec_get_rejects_out_of_bounds() {
        let mut sdk = MockStorage::new();
        let vec = StorageVec::<StorageU256>::new(U256::from(500));

        assert!(vec.get(&sdk, 0).is_none());

        vec.push(&mut sdk, U256::from(111));
        assert_eq!(vec.get(&sdk, 0).unwrap().get(&sdk), U256::from(111));
        assert!(vec.get(&sdk, 1).is_none());
        assert!(vec.get(&sdk, u64::MAX).is_none());
    }
}
