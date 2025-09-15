use crate::{
    keccak256,
    storage::{PackableCodec, StorageDescriptor, StorageLayout},
    U256,
};
use alloc::{string::String, vec::Vec};
use core::marker::PhantomData;

/// Storage map (Solidity mapping).
/// Base slot used only for computing value locations via keccak256.
#[derive(Debug, PartialEq, Eq)]
pub struct StorageMap<K, V> {
    base_slot: U256,
    _marker: PhantomData<(K, V)>,
}

// Manual Copy/Clone to avoid K,V: Copy bounds
impl<K, V> Clone for StorageMap<K, V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K, V> Copy for StorageMap<K, V> {}

impl<K, V> StorageMap<K, V> {
    pub const fn new(base_slot: U256) -> Self {
        Self {
            base_slot,
            _marker: PhantomData,
        }
    }
}

impl<K, V> StorageDescriptor for StorageMap<K, V> {
    fn new(slot: U256, offset: u8) -> Self {
        debug_assert_eq!(offset, 0, "maps always start at slot boundary");
        Self::new(slot)
    }

    fn slot(&self) -> U256 {
        self.base_slot
    }

    fn offset(&self) -> u8 {
        0
    }
}

impl<K: MapKey, V: StorageLayout> StorageMap<K, V>
where
    V::Descriptor: StorageDescriptor,
{
    /// Access value for given key.
    pub fn entry(&self, key: K) -> V::Accessor {
        let value_slot = key.compute_slot(self.base_slot);

        // Packable values start at rightmost position in slot
        let offset = if V::SLOTS == 0 {
            (32 - V::BYTES) as u8
        } else {
            0
        };

        V::access(V::Descriptor::new(value_slot, offset))
    }
}

impl<K: MapKey, V: StorageLayout> StorageLayout for StorageMap<K, V>
where
    V::Descriptor: StorageDescriptor,
{
    type Descriptor = Self;
    type Accessor = Self;

    const BYTES: usize = 32; // Base slot only
    const SLOTS: usize = 1; // Reserve one slot for hash computation

    fn access(descriptor: Self::Descriptor) -> Self::Accessor {
        descriptor
    }
}

/// Trait for types that can be used as map keys.
pub trait MapKey {
    fn compute_slot(&self, base_slot: U256) -> U256;
}

// MapKey for primitive types via PackableCodec
impl<T: PackableCodec> MapKey for T {
    fn compute_slot(&self, base_slot: U256) -> U256 {
        let mut key_bytes = [0u8; 32];

        // Right-align key in 32 bytes
        if T::ENCODED_SIZE <= 32 {
            let offset = 32 - T::ENCODED_SIZE;
            self.encode_into(&mut key_bytes[offset..]);
        }

        // keccak256(key || base_slot)
        let mut data = [0u8; 64];
        data[0..32].copy_from_slice(&key_bytes);
        data[32..64].copy_from_slice(&base_slot.to_be_bytes::<32>());

        let hash = keccak256(data);
        U256::from_be_bytes(hash.0)
    }
}

// Dynamic key types
impl MapKey for &[u8] {
    fn compute_slot(&self, base_slot: U256) -> U256 {
        let mut data = Vec::with_capacity(self.len() + 32);
        data.extend_from_slice(self);
        data.extend_from_slice(&base_slot.to_be_bytes::<32>());

        let hash = keccak256(&data);
        U256::from_be_bytes(hash.0)
    }
}

impl MapKey for Vec<u8> {
    fn compute_slot(&self, base_slot: U256) -> U256 {
        self.as_slice().compute_slot(base_slot)
    }
}

impl MapKey for &str {
    fn compute_slot(&self, base_slot: U256) -> U256 {
        self.as_bytes().compute_slot(base_slot)
    }
}

impl MapKey for String {
    fn compute_slot(&self, base_slot: U256) -> U256 {
        self.as_bytes().compute_slot(base_slot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{array::StorageArray, mock::MockStorage, primitive::StoragePrimitive};

    #[test]
    fn test_map_basic_operations() {
        let mut sdk = MockStorage::new();
        let map =
            StorageMap::<U256, StoragePrimitive<U256>>::new(U256::from(100));

        // Set values for different keys
        map.entry(U256::from(1)).set(&mut sdk, U256::from(111));
        map.entry(U256::from(2)).set(&mut sdk, U256::from(222));
        map.entry(U256::from(42)).set(&mut sdk, U256::from(424242));

        // Get values back
        assert_eq!(map.entry(U256::from(1)).get(&sdk), U256::from(111));
        assert_eq!(map.entry(U256::from(2)).get(&sdk), U256::from(222));
        assert_eq!(map.entry(U256::from(42)).get(&sdk), U256::from(424242));

        // Non-existent key returns zero
        assert_eq!(map.entry(U256::from(999)).get(&sdk), U256::ZERO);
    }

    #[test]
    fn test_map_slot_calculation() {
        // Test that slot calculation matches Solidity's keccak256(key || slot)
        let key = U256::from(7);
        let expected_slot = {
            let mut data = [0u8; 64];
            data[0..32].copy_from_slice(&key.to_be_bytes::<32>());
            data[32..64].copy_from_slice(&U256::from(5).to_be_bytes::<32>());
            let hash = keccak256(data);
            U256::from_be_bytes(hash.0)
        };

        assert_eq!(key.compute_slot(U256::from(5)), expected_slot);
    }

    #[test]
    fn test_map_with_various_key_types() {
        let mut sdk = MockStorage::new();

        // Bool keys
        let bool_map =
            StorageMap::<bool, StoragePrimitive<U256>>::new(U256::from(200));
        bool_map.entry(true).set(&mut sdk, U256::from(100));
        bool_map.entry(false).set(&mut sdk, U256::from(200));
        assert_eq!(bool_map.entry(true).get(&sdk), U256::from(100));
        assert_eq!(bool_map.entry(false).get(&sdk), U256::from(200));

        // String keys
        let string_map =
            StorageMap::<&str, StoragePrimitive<U256>>::new(U256::from(300));
        string_map.entry("alice").set(&mut sdk, U256::from(1000));
        string_map.entry("bob").set(&mut sdk, U256::from(2000));
        assert_eq!(string_map.entry("alice").get(&sdk), U256::from(1000));
        assert_eq!(string_map.entry("bob").get(&sdk), U256::from(2000));

        // u64 keys
        let u64_map =
            StorageMap::<u64, StoragePrimitive<U256>>::new(U256::from(400));
        u64_map.entry(12345u64).set(&mut sdk, U256::from(999));

        assert_eq!(u64_map.entry(12345u64).get(&sdk), U256::from(999));
    }

    #[test]
    fn test_nested_maps() {
        let mut sdk = MockStorage::new();
        // Map<U256, Map<U256, Primitive<U256>>>
        let map = StorageMap::<
            U256,
            StorageMap<U256, StoragePrimitive<U256>>,
        >::new(U256::from(500));

        // Set nested values
        map.entry(U256::from(1))
            .entry(U256::from(10))
            .set(&mut sdk, U256::from(110));

        map.entry(U256::from(1))
            .entry(U256::from(20))
            .set(&mut sdk, U256::from(120));

        map.entry(U256::from(2))
            .entry(U256::from(10))
            .set(&mut sdk, U256::from(210));

        // Get nested values
        assert_eq!(
            map.entry(U256::from(1)).entry(U256::from(10)).get(&sdk),
            U256::from(110)
        );
        assert_eq!(
            map.entry(U256::from(1)).entry(U256::from(20)).get(&sdk),
            U256::from(120)
        );
        assert_eq!(
            map.entry(U256::from(2)).entry(U256::from(10)).get(&sdk),
            U256::from(210)
        );
    }

    #[test]
    fn test_map_with_arrays_as_values() {
        let mut sdk = MockStorage::new();
        // Map<U256, Array<Primitive<u64>, 3>>
        let map =
            StorageMap::<U256, StorageArray<StoragePrimitive<u64>, 3>>::new(
                U256::from(600),
            );

        // Set array values for key 1
        let array1 = map.entry(U256::from(1));
        array1.at(0).set(&mut sdk, 100u64);
        array1.at(1).set(&mut sdk, 200u64);
        array1.at(2).set(&mut sdk, 300u64);

        // Set array values for key 2
        let array2 = map.entry(U256::from(2));
        array2.at(0).set(&mut sdk, 400u64);
        array2.at(1).set(&mut sdk, 500u64);
        array2.at(2).set(&mut sdk, 600u64);

        // Verify values
        assert_eq!(map.entry(U256::from(1)).at(0).get(&sdk), 100u64);
        assert_eq!(map.entry(U256::from(1)).at(1).get(&sdk), 200u64);
        assert_eq!(map.entry(U256::from(1)).at(2).get(&sdk), 300u64);

        assert_eq!(map.entry(U256::from(2)).at(0).get(&sdk), 400u64);
        assert_eq!(map.entry(U256::from(2)).at(1).get(&sdk), 500u64);
        assert_eq!(map.entry(U256::from(2)).at(2).get(&sdk), 600u64);
    }

    #[test]
    fn test_map_overwrites() {
        let mut sdk = MockStorage::new();
        let map =
            StorageMap::<U256, StoragePrimitive<U256>>::new(U256::from(700));

        // Set initial value
        map.entry(U256::from(42)).set(&mut sdk, U256::from(100));
        assert_eq!(map.entry(U256::from(42)).get(&sdk), U256::from(100));

        // Overwrite
        map.entry(U256::from(42)).set(&mut sdk, U256::from(200));
        assert_eq!(map.entry(U256::from(42)).get(&sdk), U256::from(200));
    }

    #[test]
    fn test_map_storage_isolation() {
        let mut sdk = MockStorage::new();

        // Two maps at different slots
        let map1 =
            StorageMap::<U256, StoragePrimitive<U256>>::new(U256::from(800));
        let map2 =
            StorageMap::<U256, StoragePrimitive<U256>>::new(U256::from(801));

        // Same key, different values
        map1.entry(U256::from(1)).set(&mut sdk, U256::from(111));
        map2.entry(U256::from(1)).set(&mut sdk, U256::from(222));

        // Values should be isolated
        assert_eq!(map1.entry(U256::from(1)).get(&sdk), U256::from(111));
        assert_eq!(map2.entry(U256::from(1)).get(&sdk), U256::from(222));
    }

    #[test]
    fn test_map_storage_layout() {
        let mut sdk = MockStorage::new();
        let map =
            StorageMap::<U256, StoragePrimitive<U256>>::new(U256::from(5));

        // Set value for key = 7
        let key = U256::from(7);
        let value = U256::from(0x123456);
        map.entry(key).set(&mut sdk, value);

        // Calculate expected slot: keccak256(key || base_slot)
        let mut data = [0u8; 64];
        data[0..32].copy_from_slice(&key.to_be_bytes::<32>());
        data[32..64].copy_from_slice(&U256::from(5).to_be_bytes::<32>());
        let expected_slot = U256::from_be_bytes(keccak256(data).0);

        // Verify value is stored at the correct slot
        assert_eq!(sdk.get_slot(expected_slot), value);

        // Verify the base slot remains empty (maps don't store data there)
        assert_eq!(sdk.get_slot(U256::from(5)), U256::ZERO);
    }

    #[test]
    fn test_map_with_packed_values() {
        let mut sdk = MockStorage::new();
        let map =
            StorageMap::<U256, StoragePrimitive<u64>>::new(U256::from(10));

        // Set u64 value for key = 1
        map.entry(U256::from(1))
            .set(&mut sdk, 0xDEADBEEFCAFEBABEu64);

        // Calculate slot
        let mut data = [0u8; 64];
        data[0..32].copy_from_slice(&U256::from(1).to_be_bytes::<32>());
        data[32..64].copy_from_slice(&U256::from(10).to_be_bytes::<32>());
        let slot = U256::from_be_bytes(keccak256(data).0);

        // u64 should be stored at the rightmost 8 bytes (offset 24)
        let stored = sdk.get_slot_hex(slot);
        assert_eq!(&stored[48..], "deadbeefcafebabe"); // Last 16 hex chars = 8 bytes
    }

    #[test]
    fn test_map_key_types_storage() {
        let mut sdk = MockStorage::new();

        // Test bool key storage
        let bool_map =
            StorageMap::<bool, StoragePrimitive<U256>>::new(U256::from(20));
        bool_map.entry(true).set(&mut sdk, U256::from(100));

        // Calculate slot for true (encoded as 1)
        let mut data = [0u8; 64];
        data[0..32].copy_from_slice(&U256::from(1).to_be_bytes::<32>());
        data[32..64].copy_from_slice(&U256::from(20).to_be_bytes::<32>());
        let slot_true = U256::from_be_bytes(keccak256(data).0);
        assert_eq!(sdk.get_slot(slot_true), U256::from(100));

        // Test string key storage
        let string_map =
            StorageMap::<&str, StoragePrimitive<U256>>::new(U256::from(30));
        string_map.entry("test").set(&mut sdk, U256::from(999));

        // Calculate slot for "test"
        let mut str_data = Vec::new();
        str_data.extend_from_slice(b"test");
        str_data.extend_from_slice(&U256::from(30).to_be_bytes::<32>());
        let slot_test = U256::from_be_bytes(keccak256(&str_data).0);
        assert_eq!(sdk.get_slot(slot_test), U256::from(999));
    }

    #[test]
    fn test_nested_maps_storage() {
        let mut sdk = MockStorage::new();
        let map = StorageMap::<
            U256,
            StorageMap<U256, StoragePrimitive<U256>>,
        >::new(U256::from(40));

        // Set map[1][2] = 100
        map.entry(U256::from(1))
            .entry(U256::from(2))
            .set(&mut sdk, U256::from(100));

        // Calculate first level slot: keccak256(1 || 40)
        let mut data1 = [0u8; 64];
        data1[0..32].copy_from_slice(&U256::from(1).to_be_bytes::<32>());
        data1[32..64].copy_from_slice(&U256::from(40).to_be_bytes::<32>());
        let slot1 = U256::from_be_bytes(keccak256(data1).0);

        // Calculate second level slot: keccak256(2 || slot1)
        let mut data2 = [0u8; 64];
        data2[0..32].copy_from_slice(&U256::from(2).to_be_bytes::<32>());
        data2[32..64].copy_from_slice(&slot1.to_be_bytes::<32>());
        let slot2 = U256::from_be_bytes(keccak256(data2).0);

        // Verify value is at the correct nested slot
        assert_eq!(sdk.get_slot(slot2), U256::from(100));
    }

    #[test]
    fn test_map_with_array_values_storage() {
        let mut sdk = MockStorage::new();
        let map =
            StorageMap::<U256, StorageArray<StoragePrimitive<u64>, 3>>::new(
                U256::from(50),
            );

        // Set array values for key = 5
        let array = map.entry(U256::from(5));
        array.at(0).set(&mut sdk, 0x1111u64);
        array.at(1).set(&mut sdk, 0x2222u64);
        array.at(2).set(&mut sdk, 0x3333u64);

        // Calculate base slot for the array
        let mut data = [0u8; 64];
        data[0..32].copy_from_slice(&U256::from(5).to_be_bytes::<32>());
        data[32..64].copy_from_slice(&U256::from(50).to_be_bytes::<32>());
        let array_slot = U256::from_be_bytes(keccak256(data).0);

        // All 3 u64 values should be packed in one slot
        // Layout: [empty(8)] [elem2(8)] [elem1(8)] [elem0(8)]
        let stored = sdk.get_slot_hex(array_slot);
        let expected = "0000000000000000000000000000333300000000000022220000000000001111";

        assert_eq!(&stored, expected);
    }

    #[test]
    fn test_map_isolation() {
        let mut sdk = MockStorage::new();

        // Two maps at different slots
        let map1 =
            StorageMap::<U256, StoragePrimitive<U256>>::new(U256::from(60));
        let map2 =
            StorageMap::<U256, StoragePrimitive<U256>>::new(U256::from(61));

        // Same key, different values
        map1.entry(U256::from(1)).set(&mut sdk, U256::from(111));
        map2.entry(U256::from(1)).set(&mut sdk, U256::from(222));

        // Calculate slots
        let mut data1 = [0u8; 64];
        data1[0..32].copy_from_slice(&U256::from(1).to_be_bytes::<32>());
        data1[32..64].copy_from_slice(&U256::from(60).to_be_bytes::<32>());
        let slot1 = U256::from_be_bytes(keccak256(data1).0);

        let mut data2 = [0u8; 64];
        data2[0..32].copy_from_slice(&U256::from(1).to_be_bytes::<32>());
        data2[32..64].copy_from_slice(&U256::from(61).to_be_bytes::<32>());
        let slot2 = U256::from_be_bytes(keccak256(data2).0);

        // Verify slots are different and contain correct values
        assert_ne!(slot1, slot2);
        assert_eq!(sdk.get_slot(slot1), U256::from(111));
        assert_eq!(sdk.get_slot(slot2), U256::from(222));
    }

    #[test]
    fn test_map_zero_key() {
        let mut sdk = MockStorage::new();
        let map =
            StorageMap::<U256, StoragePrimitive<U256>>::new(U256::from(70));

        // Test with key = 0
        map.entry(U256::ZERO).set(&mut sdk, U256::from(0xABCDEF));

        // Calculate slot for key = 0
        let mut data = [0u8; 64];
        data[0..32].copy_from_slice(&U256::ZERO.to_be_bytes::<32>());
        data[32..64].copy_from_slice(&U256::from(70).to_be_bytes::<32>());
        let slot = U256::from_be_bytes(keccak256(data).0);

        assert_eq!(sdk.get_slot(slot), U256::from(0xABCDEF));
    }
}
