use crate::{
    storage::{StorageDescriptor, StorageLayout},
    U256,
};
use alloc::{string::String, vec::Vec};
use core::marker::PhantomData;
use fluentbase_crypto::crypto_keccak256;

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

/// Solidity's `h(k)`: a fixed-size mapping key padded to one 32-byte word.
///
/// Solidity locates `mapping(K => V)` entries at `keccak256(h(k) . p)`, where `h`
/// pads the key the same way the key type is laid out in memory. That is *not* the
/// packed storage layout of [`PackableCodec`], which right-aligns everything:
///
/// | key type                   | `h(k)`                                   |
/// |----------------------------|------------------------------------------|
/// | `uintN`, `address`, `bool` | right-aligned, zero-padded on the left   |
/// | `intN`                     | right-aligned, sign-extended on the left |
/// | `bytesN`                   | left-aligned, zero-padded on the right   |
///
/// Reusing one universal layout would place negative `intN` and every `bytesN`
/// key on a different slot than Solidity, splitting state across mixed-language,
/// state-proof, and Solidity-to-rWasm upgrade boundaries. So key padding is its
/// own trait, and every key type states its alignment explicitly.
///
/// Dynamic keys (`bytes`, `string`) are hashed unpadded and implement [`MapKey`]
/// directly instead.
pub trait MapKeyCodec: Copy {
    /// Encode the key as Solidity would pad it into a 32-byte word.
    fn encode_key_word(&self) -> [u8; 32];
}

impl<T: MapKeyCodec> MapKey for T {
    fn compute_slot(&self, base_slot: U256) -> U256 {
        // keccak256(h(key) || base_slot)
        let mut data = [0u8; 64];
        data[0..32].copy_from_slice(&self.encode_key_word());
        data[32..64].copy_from_slice(&base_slot.to_be_bytes::<32>());

        let hash = crypto_keccak256(data);
        U256::from_be_bytes(hash.0)
    }
}

// Dynamic key types
impl MapKey for &[u8] {
    fn compute_slot(&self, base_slot: U256) -> U256 {
        let mut data = Vec::with_capacity(self.len() + 32);
        data.extend_from_slice(self);
        data.extend_from_slice(&base_slot.to_be_bytes::<32>());

        let hash = crypto_keccak256(data);
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
        let map = StorageMap::<U256, StoragePrimitive<U256>>::new(U256::from(100));

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
            let hash = crypto_keccak256(data);
            U256::from_be_bytes(hash.0)
        };

        assert_eq!(key.compute_slot(U256::from(5)), expected_slot);
    }

    #[test]
    fn test_map_with_various_key_types() {
        let mut sdk = MockStorage::new();

        // Bool keys
        let bool_map = StorageMap::<bool, StoragePrimitive<U256>>::new(U256::from(200));
        bool_map.entry(true).set(&mut sdk, U256::from(100));
        bool_map.entry(false).set(&mut sdk, U256::from(200));
        assert_eq!(bool_map.entry(true).get(&sdk), U256::from(100));
        assert_eq!(bool_map.entry(false).get(&sdk), U256::from(200));

        // String keys
        let string_map = StorageMap::<&str, StoragePrimitive<U256>>::new(U256::from(300));
        string_map.entry("alice").set(&mut sdk, U256::from(1000));
        string_map.entry("bob").set(&mut sdk, U256::from(2000));
        assert_eq!(string_map.entry("alice").get(&sdk), U256::from(1000));
        assert_eq!(string_map.entry("bob").get(&sdk), U256::from(2000));

        // u64 keys
        let u64_map = StorageMap::<u64, StoragePrimitive<U256>>::new(U256::from(400));
        u64_map.entry(12345u64).set(&mut sdk, U256::from(999));

        assert_eq!(u64_map.entry(12345u64).get(&sdk), U256::from(999));
    }

    #[test]
    fn test_nested_maps() {
        let mut sdk = MockStorage::new();
        // Map<U256, Map<U256, Primitive<U256>>>
        let map =
            StorageMap::<U256, StorageMap<U256, StoragePrimitive<U256>>>::new(U256::from(500));

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
        let map = StorageMap::<U256, StorageArray<StoragePrimitive<u64>, 3>>::new(U256::from(600));

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
        let map = StorageMap::<U256, StoragePrimitive<U256>>::new(U256::from(700));

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
        let map1 = StorageMap::<U256, StoragePrimitive<U256>>::new(U256::from(800));
        let map2 = StorageMap::<U256, StoragePrimitive<U256>>::new(U256::from(801));

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
        let map = StorageMap::<U256, StoragePrimitive<U256>>::new(U256::from(5));

        // Set value for key = 7
        let key = U256::from(7);
        let value = U256::from(0x123456);
        map.entry(key).set(&mut sdk, value);

        // Calculate expected slot: keccak256(key || base_slot)
        let mut data = [0u8; 64];
        data[0..32].copy_from_slice(&key.to_be_bytes::<32>());
        data[32..64].copy_from_slice(&U256::from(5).to_be_bytes::<32>());
        let expected_slot = U256::from_be_bytes(crypto_keccak256(data).0);

        // Verify value is stored at the correct slot
        assert_eq!(sdk.get_slot(expected_slot), value);

        // Verify the base slot remains empty (maps don't store data there)
        assert_eq!(sdk.get_slot(U256::from(5)), U256::ZERO);
    }

    #[test]
    fn test_map_with_packed_values() {
        let mut sdk = MockStorage::new();
        let map = StorageMap::<U256, StoragePrimitive<u64>>::new(U256::from(10));

        // Set u64 value for key = 1
        map.entry(U256::from(1))
            .set(&mut sdk, 0xDEADBEEFCAFEBABEu64);

        // Calculate slot
        let mut data = [0u8; 64];
        data[0..32].copy_from_slice(&U256::from(1).to_be_bytes::<32>());
        data[32..64].copy_from_slice(&U256::from(10).to_be_bytes::<32>());
        let slot = U256::from_be_bytes(crypto_keccak256(data).0);

        // u64 should be stored at the rightmost 8 bytes (offset 24)
        let stored = sdk.get_slot_hex(slot);
        assert_eq!(&stored[48..], "deadbeefcafebabe"); // Last 16 hex chars = 8 bytes
    }

    #[test]
    fn test_map_key_types_storage() {
        let mut sdk = MockStorage::new();

        // Test bool key storage
        let bool_map = StorageMap::<bool, StoragePrimitive<U256>>::new(U256::from(20));
        bool_map.entry(true).set(&mut sdk, U256::from(100));

        // Calculate slot for true (encoded as 1)
        let mut data = [0u8; 64];
        data[0..32].copy_from_slice(&U256::from(1).to_be_bytes::<32>());
        data[32..64].copy_from_slice(&U256::from(20).to_be_bytes::<32>());
        let slot_true = U256::from_be_bytes(crypto_keccak256(data).0);
        assert_eq!(sdk.get_slot(slot_true), U256::from(100));

        // Test string key storage
        let string_map = StorageMap::<&str, StoragePrimitive<U256>>::new(U256::from(30));
        string_map.entry("test").set(&mut sdk, U256::from(999));

        // Calculate slot for "test"
        let mut str_data = Vec::new();
        str_data.extend_from_slice(b"test");
        str_data.extend_from_slice(&U256::from(30).to_be_bytes::<32>());
        let slot_test = U256::from_be_bytes(crypto_keccak256(str_data).0);
        assert_eq!(sdk.get_slot(slot_test), U256::from(999));
    }

    #[test]
    fn test_nested_maps_storage() {
        let mut sdk = MockStorage::new();
        let map = StorageMap::<U256, StorageMap<U256, StoragePrimitive<U256>>>::new(U256::from(40));

        // Set map[1][2] = 100
        map.entry(U256::from(1))
            .entry(U256::from(2))
            .set(&mut sdk, U256::from(100));

        // Calculate first level slot: keccak256(1 || 40)
        let mut data1 = [0u8; 64];
        data1[0..32].copy_from_slice(&U256::from(1).to_be_bytes::<32>());
        data1[32..64].copy_from_slice(&U256::from(40).to_be_bytes::<32>());
        let slot1 = U256::from_be_bytes(crypto_keccak256(data1).0);

        // Calculate second level slot: keccak256(2 || slot1)
        let mut data2 = [0u8; 64];
        data2[0..32].copy_from_slice(&U256::from(2).to_be_bytes::<32>());
        data2[32..64].copy_from_slice(&slot1.to_be_bytes::<32>());
        let slot2 = U256::from_be_bytes(crypto_keccak256(data2).0);

        // Verify value is at the correct nested slot
        assert_eq!(sdk.get_slot(slot2), U256::from(100));
    }

    #[test]
    fn test_map_with_array_values_storage() {
        let mut sdk = MockStorage::new();
        let map = StorageMap::<U256, StorageArray<StoragePrimitive<u64>, 3>>::new(U256::from(50));

        // Set array values for key = 5
        let array = map.entry(U256::from(5));
        array.at(0).set(&mut sdk, 0x1111u64);
        array.at(1).set(&mut sdk, 0x2222u64);
        array.at(2).set(&mut sdk, 0x3333u64);

        // Calculate base slot for the array
        let mut data = [0u8; 64];
        data[0..32].copy_from_slice(&U256::from(5).to_be_bytes::<32>());
        data[32..64].copy_from_slice(&U256::from(50).to_be_bytes::<32>());
        let array_slot = U256::from_be_bytes(crypto_keccak256(data).0);

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
        let map1 = StorageMap::<U256, StoragePrimitive<U256>>::new(U256::from(60));
        let map2 = StorageMap::<U256, StoragePrimitive<U256>>::new(U256::from(61));

        // Same key, different values
        map1.entry(U256::from(1)).set(&mut sdk, U256::from(111));
        map2.entry(U256::from(1)).set(&mut sdk, U256::from(222));

        // Calculate slots
        let mut data1 = [0u8; 64];
        data1[0..32].copy_from_slice(&U256::from(1).to_be_bytes::<32>());
        data1[32..64].copy_from_slice(&U256::from(60).to_be_bytes::<32>());
        let slot1 = U256::from_be_bytes(crypto_keccak256(data1).0);

        let mut data2 = [0u8; 64];
        data2[0..32].copy_from_slice(&U256::from(1).to_be_bytes::<32>());
        data2[32..64].copy_from_slice(&U256::from(61).to_be_bytes::<32>());
        let slot2 = U256::from_be_bytes(crypto_keccak256(data2).0);

        // Verify slots are different and contain correct values
        assert_ne!(slot1, slot2);
        assert_eq!(sdk.get_slot(slot1), U256::from(111));
        assert_eq!(sdk.get_slot(slot2), U256::from(222));
    }

    /// Mapping-key slots cross-checked against the Solidity compiler.
    ///
    /// Every expected slot below was produced by solc 0.8.34, not by this
    /// implementation. Each key type is declared in a contract whose slot 0 is an
    /// unused padding word, so `mapping(intN => uint256)` and
    /// `mapping(uintN => uint256)` sit at base slot `N / 8` and
    /// `mapping(bytesN => uint256)` at base slot `N`. A separate contract holds
    /// `address` at slot 1, `bool` at 2,
    /// `mapping(address => mapping(address => uint256))` at 3,
    /// `mapping(int24 => mapping(bytes4 => uint256))` at 4, `string` at 5 and
    /// `bytes` at 6. A forge test calls each compiler-generated getter under
    /// `vm.record()` and reports the slot the compiled code actually loads.
    mod solidity_vectors {
        use super::*;
        use crate::{hex, storage::StoragePrimitive, Address, FixedBytes, Signed, Uint};

        /// `bytesN` vectors use the first `N` bytes of this pattern as the key.
        const PATTERN: [u8; 32] = [
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
            0xff, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc,
            0xdd, 0xee, 0xff, 0x01,
        ];

        /// `0x00112233445566778899aabbccddeeff00112233`
        const ADDR_A: [u8; 20] = [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff, 0x00, 0x11, 0x22, 0x33,
        ];

        /// `0xffeeddccbbaa99887766554433221100ffeeddcc`
        const ADDR_B: [u8; 20] = [
            0xff, 0xee, 0xdd, 0xcc, 0xbb, 0xaa, 0x99, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22,
            0x11, 0x00, 0xff, 0xee, 0xdd, 0xcc,
        ];

        /// Parses a reference slot recorded from Solidity.
        fn expect(slot_hex: &str) -> U256 {
            let bytes = hex::decode(slot_hex.strip_prefix("0x").unwrap()).expect("invalid vector");
            U256::from_be_bytes::<32>(bytes.try_into().expect("vector must be 32 bytes"))
        }

        macro_rules! check_signed {
            ($($bits:literal, $limbs:literal =>
                minus_one: $minus_one:literal,
                min: $min:literal,
                one: $one:literal;)*) => {
                $({
                    let base = U256::from($bits / 8);
                    assert_eq!(
                        Signed::<$bits, $limbs>::MINUS_ONE.compute_slot(base),
                        expect($minus_one),
                        "int{} key -1",
                        $bits,
                    );
                    assert_eq!(
                        Signed::<$bits, $limbs>::MIN.compute_slot(base),
                        expect($min),
                        "int{} key MIN",
                        $bits,
                    );
                    assert_eq!(
                        Signed::<$bits, $limbs>::ONE.compute_slot(base),
                        expect($one),
                        "int{} key 1",
                        $bits,
                    );
                })*
            };
        }

        macro_rules! check_unsigned {
            ($($bits:literal, $limbs:literal =>
                one: $one:literal,
                max: $max:literal;)*) => {
                $({
                    let base = U256::from($bits / 8);
                    assert_eq!(
                        Uint::<$bits, $limbs>::ONE.compute_slot(base),
                        expect($one),
                        "uint{} key 1",
                        $bits,
                    );
                    assert_eq!(
                        Uint::<$bits, $limbs>::MAX.compute_slot(base),
                        expect($max),
                        "uint{} key MAX",
                        $bits,
                    );
                })*
            };
        }

        macro_rules! check_fixed_bytes {
            ($($n:literal => $slot:literal;)*) => {
                $({
                    let key = FixedBytes::<$n>::from_slice(&PATTERN[..$n]);
                    assert_eq!(
                        key.compute_slot(U256::from($n)),
                        expect($slot),
                        "bytes{} key",
                        $n,
                    );
                })*
            };
        }

        #[test]
        fn signed_keys_are_sign_extended_like_solidity() {
            check_signed! {
                8, 1 =>
                    minus_one: "0xc39d774f18115b85b81494d65e588b565d73abc969333d1da7b0a0eb0729accd",
                    min:       "0xa6448894d065a3e7161c6293c2db5e883893c02ad215165c869901858b9036a9",
                    one:       "0xcc69885fda6bcc1a4ace058b4a62bf5e179ea78fd58a1ccd71c22cc9b688792f";
                16, 1 =>
                    minus_one: "0x38b5b2ceac7637132d27514ffcf440b705287635075af7b8bd5adcaa6a4cc5bb",
                    min:       "0xcf2fb756914b19221bf2fef06148e4b4b9392b9c9a1f0bc7d93709ef28342ba5",
                    one:       "0xe90b7bceb6e7df5418fb78d8ee546e97c83a08bbccc01a0644d599ccd2a7c2e0";
                24, 1 =>
                    minus_one: "0xb1ee3b3d0d99532dd9f14b22c0b908d4eec0e052c3827bbed2d6c3986954d08c",
                    min:       "0x7f43945837635d09673421abd13cee36348137796c27cd8bd12ca4525a0df5e2",
                    one:       "0xa15bc60c955c405d20d9149c709e2460f1c2d9a497496a7f46004d1772c3054c";
                32, 1 =>
                    minus_one: "0xd8c80a9840ed58f33f2186a8fbc29ecd8c3610d196f1da047301bd51988eb95c",
                    min:       "0x6e55b48d4744edeccafa886884bc9a4dbd00a777c2dcd5791e8b167088622824",
                    one:       "0xabd6e7cb50984ff9c2f3e18a2660c3353dadf4e3291deeb275dae2cd1e44fe05";
                40, 1 =>
                    minus_one: "0x2e8de2577e7c560a9913fd732cd5ba1f61f809b10c283800da9499091ac562a5",
                    min:       "0x57a63a1072d76f72468c74cfd819530d2f5b24d1e3e247400e5ccfa7d3bd936f",
                    one:       "0x1471eb6eb2c5e789fc3de43f8ce62938c7d1836ec861730447e2ada8fd81017b";
                48, 1 =>
                    minus_one: "0x63187d71e139eee983a88d0737447c7451979b3dbb75903c76b5fe430d36588e",
                    min:       "0x135966d89c951315b28d6b29b104e8511bea5a257c171d75feae6da900c60615",
                    one:       "0x3e5fec24aa4dc4e5aee2e025e51e1392c72a2500577559fae9665c6d52bd6a31";
                56, 1 =>
                    minus_one: "0xa79741ff9376312d805b646fe98c5eaaa690c33d0a4c18cb1d87dfa9e9a9af0b",
                    min:       "0x6c08c200821b9f7623e3b9bfd71a73518d5dfc013abeb7aaccb7895bb3a93795",
                    one:       "0xb39221ace053465ec3453ce2b36430bd138b997ecea25c1043da0c366812b828";
                64, 1 =>
                    minus_one: "0x50015d2c5500ee864adc0ae35838917a2d9a98eb2ab97342b4d689d9a074dbfa",
                    min:       "0xb6f2a08fd84fd72638abdb192e8c8e97b6d535d5052c4a0421762dcd1a7379cb",
                    one:       "0xad67d757c34507f157cacfa2e3153e9f260a2244f30428821be7be64587ac55f";
                72, 2 =>
                    minus_one: "0x8be17021fa7918486222bbb1bc9d45bbdf93d7f49d8066170141bb3a10b823f4",
                    min:       "0x5142a8cc62fa4971670907ac3dc698d9c16246254da7df61811d43a2bbbea5ca",
                    one:       "0x92e85d02570a8092d09a6e3a57665bc3815a2699a4074001bf1ccabf660f5a36";
                80, 2 =>
                    minus_one: "0x1cde448d8c1d4666ae6874ced948f1e0ad12a4bed8302f3be564e5fd7540b8eb",
                    min:       "0x86101a743af0d237ba40bd3fdf92ca810b6afc8d27f17d39fa77dfcea754b088",
                    one:       "0xbbc70db1b6c7afd11e79c0fb0051300458f1a3acb8ee9789d9b6b26c61ad9bc7";
                88, 2 =>
                    minus_one: "0x5030e2ff9d3404671c17fc42c5714d9bdf34dbce2663e34014ba8c942df513fb",
                    min:       "0xd62a59a1c880f5e3c9691388aac3294aa9a50a92199bf7abb89c7ac98b6b3cb3",
                    one:       "0x72c6bfb7988af3a1efa6568f02a999bc52252641c659d85961ca3d372b57d5cf";
                96, 2 =>
                    minus_one: "0xa10f30cc98fff5708c93df4a220b8f117034e197ce203f377237571fbe641ae7",
                    min:       "0xabd93e15c426f9233215b50d1fb0238f25cebf5780846a88e6625af42059adb9",
                    one:       "0xd421a5181c571bba3f01190c922c3b2a896fc1d84e86c9f17ac10e67ebef8b5c";
                104, 2 =>
                    minus_one: "0xc84cd90342df8739b373f7be527807c208a84569afc12be2cb8f5c5052dfb349",
                    min:       "0x5625d07e9a3b7082acb0a43ba1bef59b763540dd5536b76a44ddd0f6fd0c1268",
                    one:       "0xfd54ff1ed53f34a900b24c5ba64f85761163b5d82d98a47b9bd80e45466993c5";
                112, 2 =>
                    minus_one: "0x486cf3b7204a0f1112420044e95a29440d09fee9d3a9392854ddf6d046c953b3",
                    min:       "0x757bc3c4a1c49a515fa968225656a81b6079a27f10075e8471571b79a5166c53",
                    one:       "0xa7c5ba7114a813b50159add3a36832908dc83db71d0b9a24c2ad0f83be958207";
                120, 2 =>
                    minus_one: "0x5f49b1b339959b31ad74f07b88b44259ea11bfb77e53c425ee4e04612cace647",
                    min:       "0xd92dd9dcf7ded059da2f95bceb2d17f1316abce458cb99f3a8de1a7a3559efd5",
                    one:       "0x169f97de0d9a84d840042b17d3c6b9638b3d6fd9024c9eb0c7a306a17b49f88f";
                128, 2 =>
                    minus_one: "0x67c618532631f5e38a3b9c5e06a3e5553e5fac44409c6f6f7364f2525b56773a",
                    min:       "0x5a065330214f8c8ac10a99d46ab28c8765aec8bb0237b9ab0d1086283ca05320",
                    one:       "0x8c6065603763fec3f5742441d3833f3f43b982453612d76adb39a885e3006b5f";
                136, 3 =>
                    minus_one: "0xa69f8edf3b946707c160fd4b4533bf1626bacaec5eefa8c07ba416ea6a23adb0",
                    min:       "0x361bd2d8a4a72401470b71335f24667611e4070a53206b822f57ab640f4800b0",
                    one:       "0x17bc176d2408558f6e4111feebc3cab4e16b63e967be91cde721f4c8a488b552";
                144, 3 =>
                    minus_one: "0x38a7014c891815c312752673b617180a4abd1a642674354e7719ef6c24c13037",
                    min:       "0x8afb98d9b835ce67313f66fa35cb4ff343d480b500520615b8a222babf9b0ee3",
                    one:       "0x71a67924699a20698523213e55fe499d539379d7769cd5567e2c45d583f815a3";
                152, 3 =>
                    minus_one: "0x84b7e90e34a243706436e6c933eda22efd83d670717692a842a132fb5d4f8d7c",
                    min:       "0x7877698bca6fb5ad0219127ce4b6365a8aaa7d1c97fa9f00cd0fd824eb188e85",
                    one:       "0x4155c2f711f2cdd34f8262ab8fb9b7020a700fe7b6948222152f7670d1fdf34d";
                160, 3 =>
                    minus_one: "0x435ceb1ad05f2d2be0f6b7fbfab2b8d011eee8ed51a9b37c03338442a93ceef6",
                    min:       "0xc83f523ca9f6ea8e8bf8c79a44a16be4798489c266110ad6839f7d2367b31d93",
                    one:       "0xb6c61a840592cc84133e4b25bd509abf4659307c57b160799b38490a5aa48f2c";
                168, 3 =>
                    minus_one: "0x9f0c13364c3666fa97786132804bacda0065140b4d45b1f05fcdaffca8e4c926",
                    min:       "0x4b4e0a10ba1fe0cc821b7e96b3b0de28068b5d977d1f7ceeaf9e66b2647ffee7",
                    one:       "0x27739e4bb5e6f8b5e4b57a047dca8767cc9b982a011081e086cbb0dfa9de818d";
                176, 3 =>
                    minus_one: "0x7c383b0a2168581fe8dfa3f696746b1d84f1675fb4959108456d60162848933c",
                    min:       "0x386ec75a78bdad7adec7c75deb87238f88b2d71cae09b8b59dd7e7949c847a3b",
                    one:       "0x4c4dc693d7db52f85fe052106f4b4b920e78e8ef37dee82878a60ab8585faf49";
                184, 3 =>
                    minus_one: "0x6488e0c85a2670bdd10614c45b24da372bbfe3fc4b4ecffa8b7d65945a3f7e33",
                    min:       "0xe0439e8fbce049c687ff15271196b85d5f92997efe5040d090ce16358a7c6ce6",
                    one:       "0xf36d6bc9642eb6fb6ee9998b09ce990566df752ab06e11f8de7ab633bbd57b8f";
                192, 3 =>
                    minus_one: "0x399d0a24b148009d3f8925693da45cc0775b49707d17362b0daea06e36536938",
                    min:       "0xbec190a481d6fd1937a0e956d8411f1910c2e548faf2fe353385c2bdf5ded5e1",
                    one:       "0xf3794665d3af9b6fb6f858b70185898134f96768ef31c325d52e04f0ac195a4d";
                200, 4 =>
                    minus_one: "0x0d7f73ba8afef39cfc7064e55120c450f3d9a24fe3988ad6d7c5309656095f0a",
                    min:       "0xb9a59b292632ae7335a7f493f0a4646384cd1fde8da16a9603c32dee11ab93b7",
                    one:       "0xfc941c3961fb6541da34150022cddf959da0fb2353866a6bfbd249c2da092914";
                208, 4 =>
                    minus_one: "0x327c4029158af2da36a42cb8f96d218a24f8825507f9c7cab6ed11be5135155e",
                    min:       "0x0b033076351988b1f7d2a8891d7a749fc8ef53714348669eb2ce7b786bf9e1de",
                    one:       "0xf88cd8d612926ebb404e40725c01084b6e9b3ce0344cde068570342cbd448c61";
                216, 4 =>
                    minus_one: "0xaaf94f6ea6d0e15c9806381e0c5077d02aa78581851e36902a30ccea61d9f2fb",
                    min:       "0x05c76157f46a731299afa0609d0eb68b3f7c89f55e9aef10f4b4fb28fc1ad2bd",
                    one:       "0x9fafca4c9c0d5c2cbf85f49fd8ab8212430ce78c2a0cb75b51e0f9c4f9ace003";
                224, 4 =>
                    minus_one: "0x9a12beb065eada6943065a3ff2e4e903c520ca533ba60d47c7b9c3f800bd750c",
                    min:       "0x1f129cec89a15894e0a64c14c4872f900cc574731ead4454e2ce836039fc9df3",
                    one:       "0x6de76108811faf2f94afbe5ac6c98e8393206cd093932de1fbfd61bbeec43a02";
                232, 4 =>
                    minus_one: "0xc736243efc8b4b64d465443e7c5628e1e24fbb99ca2384861dddb1ebbe7e8267",
                    min:       "0x8416e4d1a17e9fbba75b05ce6d213263484afec3bb6facabfad9cd34a7587631",
                    one:       "0x9de6abd965d55c3bb0cdbf6fa175050624c6ff8fe86f682dc08f2a450ede2278";
                240, 4 =>
                    minus_one: "0xfd813bec61c7c0cea56a4053999cb1b40a70c64e6e3060d7978d40d9ef4991bb",
                    min:       "0x7e5f2308925f19ef1c3bb4e1e1ad1149ceb9e6aa82698ab7112d48d60d3120df",
                    one:       "0x873299c6a6c39b8b92f01922bb622df4a3236ea2876aac2da76f6c092cf7e98f";
                248, 4 =>
                    minus_one: "0x719ec84beffbf4a755745bc9a7e34eb5cf8ed94f6924a2e33811d832d938b4b5",
                    min:       "0x1410eb8899aa298a78b63eb9c4c588a952b59d0f4867cda76cd24b0423305c26",
                    one:       "0x820fef5837650fa3b8e45045b88059d8deaf0810350ec511c47ef768a28c2c9b";
                256, 4 =>
                    minus_one: "0x1cee60e39b32a4541f1091f87056012cddcb97999dbd2368e30c2354007ec737",
                    min:       "0x39a3aec1ef2b24058158e9609c2ad332ee9bda7e4ea237ba1c4f9e8777f1da6d",
                    one:       "0x156774b33c8bc7cb83eda4cbc43b36c7c9490ff8913c488ccd5132cfc71344ea";
            }
        }

        #[test]
        fn unsigned_keys_are_zero_extended_like_solidity() {
            check_unsigned! {
                8, 1 =>
                    one: "0xcc69885fda6bcc1a4ace058b4a62bf5e179ea78fd58a1ccd71c22cc9b688792f",
                    max: "0x24a9e90595537a4321bf3a8fd43f02c179fe79a94dde54a8c1a057e2967a4d0b";
                16, 1 =>
                    one: "0xe90b7bceb6e7df5418fb78d8ee546e97c83a08bbccc01a0644d599ccd2a7c2e0",
                    max: "0x695395ec6a2c9d3a74f1c3d78bda956395489a2ec3ce495c3087446ede7bcc9d";
                24, 1 =>
                    one: "0xa15bc60c955c405d20d9149c709e2460f1c2d9a497496a7f46004d1772c3054c",
                    max: "0x718d5407b17d62afeebaa1cf05ee0d79f9c7fd9d1af5f32d660e507ee3646281";
                32, 1 =>
                    one: "0xabd6e7cb50984ff9c2f3e18a2660c3353dadf4e3291deeb275dae2cd1e44fe05",
                    max: "0x2484d89f5223ef64143beaf0a743715076229ba3cba1b81333f195bbd499c493";
                40, 1 =>
                    one: "0x1471eb6eb2c5e789fc3de43f8ce62938c7d1836ec861730447e2ada8fd81017b",
                    max: "0x860d2b901dd1865e65ce65feadabdd4bf4261744a4f7cca964c9134b799b5f3f";
                48, 1 =>
                    one: "0x3e5fec24aa4dc4e5aee2e025e51e1392c72a2500577559fae9665c6d52bd6a31",
                    max: "0xb3304aff9a7e727966dec372983e06a978d7b53336011327d28f721b162ecefd";
                56, 1 =>
                    one: "0xb39221ace053465ec3453ce2b36430bd138b997ecea25c1043da0c366812b828",
                    max: "0x09658940dc07950a96aebbddd25447b8b5f800c210571b206f52775408ce6e50";
                64, 1 =>
                    one: "0xad67d757c34507f157cacfa2e3153e9f260a2244f30428821be7be64587ac55f",
                    max: "0x0b7244c513877c415f365e3266d1f5b7b85386b2438257e95327690b197b4523";
                72, 2 =>
                    one: "0x92e85d02570a8092d09a6e3a57665bc3815a2699a4074001bf1ccabf660f5a36",
                    max: "0xb736b02a15d164f558824358adc5b6574a53d00def4ef517f5feb50e14e447e7";
                80, 2 =>
                    one: "0xbbc70db1b6c7afd11e79c0fb0051300458f1a3acb8ee9789d9b6b26c61ad9bc7",
                    max: "0xf0cfa32fa27376ddfd98a9d7e448b70e3ca96b510e44a94af08d96e6cb4355e8";
                88, 2 =>
                    one: "0x72c6bfb7988af3a1efa6568f02a999bc52252641c659d85961ca3d372b57d5cf",
                    max: "0x1c9ad027e949c8558ca0b9cfbe9321a43a6dc3c8025ec79bb8274e18f36f0d01";
                96, 2 =>
                    one: "0xd421a5181c571bba3f01190c922c3b2a896fc1d84e86c9f17ac10e67ebef8b5c",
                    max: "0x8118b5322632f7d46ecb6b2da464f1fbf0caf4ab099821f1f1ab3d81f3fbabbd";
                104, 2 =>
                    one: "0xfd54ff1ed53f34a900b24c5ba64f85761163b5d82d98a47b9bd80e45466993c5",
                    max: "0xdff27520ef1cea0797109ce6edf36570a0db5647650f533191e196a859ca6785";
                112, 2 =>
                    one: "0xa7c5ba7114a813b50159add3a36832908dc83db71d0b9a24c2ad0f83be958207",
                    max: "0x3569203d6d22d565dad3f082b918a10628b2eb8a3f630d286645e9adb55b4667";
                120, 2 =>
                    one: "0x169f97de0d9a84d840042b17d3c6b9638b3d6fd9024c9eb0c7a306a17b49f88f",
                    max: "0x2a5c4fc39620558873ff0ca91ef8042e0874e9f22afbbacebe38117c09d5065c";
                128, 2 =>
                    one: "0x8c6065603763fec3f5742441d3833f3f43b982453612d76adb39a885e3006b5f",
                    max: "0x36da9b7b57d3afaa3e6cc456e64eaa514b2ae0d30afc92ea1ec06b30e88959ab";
                136, 3 =>
                    one: "0x17bc176d2408558f6e4111feebc3cab4e16b63e967be91cde721f4c8a488b552",
                    max: "0x57936fb2401e866936d5b5695657b1670cf656fca1bee51e67e4491404526062";
                144, 3 =>
                    one: "0x71a67924699a20698523213e55fe499d539379d7769cd5567e2c45d583f815a3",
                    max: "0x9ef5f3e12c03008c67378b967164e0161f5f1d6ae34f6df01b9378e2b80beadc";
                152, 3 =>
                    one: "0x4155c2f711f2cdd34f8262ab8fb9b7020a700fe7b6948222152f7670d1fdf34d",
                    max: "0xe5f97db8198f87a655acf93d438efde235df6dd6ebb20907d6dc820d5f796456";
                160, 3 =>
                    one: "0xb6c61a840592cc84133e4b25bd509abf4659307c57b160799b38490a5aa48f2c",
                    max: "0xeef591571549cf9b667da255ab4bb2a90dfdcb77845d7bc32c3bf4528eee03db";
                168, 3 =>
                    one: "0x27739e4bb5e6f8b5e4b57a047dca8767cc9b982a011081e086cbb0dfa9de818d",
                    max: "0x686ab6ea74c6cdd8d7a5a648013643a6089d7e22d717fcb2ac5dbd8f399efa46";
                176, 3 =>
                    one: "0x4c4dc693d7db52f85fe052106f4b4b920e78e8ef37dee82878a60ab8585faf49",
                    max: "0x8c47b3ab20072ae7cdac9c75ff8dc45df62976a10a42e4e64a81c8ffa7b28da8";
                184, 3 =>
                    one: "0xf36d6bc9642eb6fb6ee9998b09ce990566df752ab06e11f8de7ab633bbd57b8f",
                    max: "0xab8392575d39e8d03687a5ef4e7737e5c8d9ec34707ef18d29959fd21970ac42";
                192, 3 =>
                    one: "0xf3794665d3af9b6fb6f858b70185898134f96768ef31c325d52e04f0ac195a4d",
                    max: "0x027dd54657a2f84ad9e3a6e71d350c80b0d312217186b9864e9888fc992b7768";
                200, 4 =>
                    one: "0xfc941c3961fb6541da34150022cddf959da0fb2353866a6bfbd249c2da092914",
                    max: "0x9c54dd50d8ee03455137bfa590f54346660cc892134e199e5ec1fec01a75ca76";
                208, 4 =>
                    one: "0xf88cd8d612926ebb404e40725c01084b6e9b3ce0344cde068570342cbd448c61",
                    max: "0x44e8583e57d9bd6020882773d64cd6cad8c3a1b1a6041765b2b65abf45523c70";
                216, 4 =>
                    one: "0x9fafca4c9c0d5c2cbf85f49fd8ab8212430ce78c2a0cb75b51e0f9c4f9ace003",
                    max: "0x2151d824bbd7f3507ba0b5bb2bb48e15583b93b62a0eac1ed6b8f03bf2a7e709";
                224, 4 =>
                    one: "0x6de76108811faf2f94afbe5ac6c98e8393206cd093932de1fbfd61bbeec43a02",
                    max: "0x01ed95526d43b72addeec4f9ab6c4aeb19ceee53677af91de7bb975394286e51";
                232, 4 =>
                    one: "0x9de6abd965d55c3bb0cdbf6fa175050624c6ff8fe86f682dc08f2a450ede2278",
                    max: "0xb203bc3d27897c0bf469cc2ad67d0630c7c227efd02cf9ae38c4d4f68c8d463f";
                240, 4 =>
                    one: "0x873299c6a6c39b8b92f01922bb622df4a3236ea2876aac2da76f6c092cf7e98f",
                    max: "0x6910e3949173da8fdaf64e896cb500f1a4f7ff86db75644ab9817f15a26075a1";
                248, 4 =>
                    one: "0x820fef5837650fa3b8e45045b88059d8deaf0810350ec511c47ef768a28c2c9b",
                    max: "0x4932347b43efb2bde30d2fe9b51cdf92645c8ca102ca267425da755c76269ffc";
                256, 4 =>
                    one: "0x156774b33c8bc7cb83eda4cbc43b36c7c9490ff8913c488ccd5132cfc71344ea",
                    max: "0x1cee60e39b32a4541f1091f87056012cddcb97999dbd2368e30c2354007ec737";
            }
        }

        #[test]
        fn fixed_bytes_keys_are_left_aligned_like_solidity() {
            check_fixed_bytes! {
                1 => "0x5fe77fe1715fa199167260892560bdfea7e3beca6538b14e3cebe1c4589fdc43";
                2 => "0x40a782713f2a3a2841a641caf541d0994d36ff6ea1a60eaa1288ac0d11b4f63f";
                3 => "0x16eedc1fe3f8b776707c3ddb3c97201d29815a2f8012a46e44acd8af119a503e";
                4 => "0xe4489ab818d06fb7da5f73f86bb1c6d2abd37a194e7cb70804ef28265fa7a9ab";
                5 => "0xa9f0230be17a475d0a20d504ec12af13f6cb1bc73ae783c0ba6cc8c7c11a1e1f";
                6 => "0xc543d1c87654074c97398ed772fce557c63ca577aad067ab67876aebb88831fd";
                7 => "0xc4d86d1b22ec90707cb87c37b247b1efb4b4e61c931be0a2a1d2d42ab734ad3e";
                8 => "0x8b055d49df0ec9075f57a156d7cae08d3f36d1196cbfa691fa281f8e1812a06e";
                9 => "0x74420e9e5e968bbb355f2022e6302b1485324fb1b8c15543f0fc55507bedaffd";
                10 => "0x98e7c97affe4f9f51cb0dd3934578d0675c1431f4bba47f97d27d0d0877f2c04";
                11 => "0xc2b0e1b47476a33d86a6294b4e4720e9b3cdc679d2876552085bd385d08504da";
                12 => "0x2e85ed8801d95db48ad451d263fcd2147108e4bcd6f37863f9595558f65387d8";
                13 => "0x492ea390ccef3421809b276cbd0df7e89c68c408d72c0c200b3f30340a8117de";
                14 => "0x5f214951563d58225ed61330c65558c491692c06c15980fbd53b39495463c58d";
                15 => "0x8193b817808b613a1308631ba6b39cb0cd15c952596400689e29fdf284b4305c";
                16 => "0x8cf51f27d73d21467722f0bed7d16afc4f4db350efece39fd9c6581fe608f196";
                17 => "0x81e63e95cf4c7d6680478c0dca20a3a44216a6b3beac9aaa9acf3310785705cd";
                18 => "0x451c1b1ec7bf3897524ebba61e030d18b18e0d6f70020bab78e6020eb453d98e";
                19 => "0x227d3eded325e4295c9f615c8d2248808b0102562394e2d6020eafaec450cb94";
                20 => "0xfae520aea0558aedacb48cb853e9c6886ae2da5a3ce147d2f37404522bf248be";
                21 => "0xafa44a56a258e51747727e5c6e5bcd9e632dd0fb5e084cb64fb403a221025113";
                22 => "0xd6d1a238bd4a71d6e29b466cf7468fee2da0b1fc1cae2a6909f7838e28b03d83";
                23 => "0x1cf7eb14d69eb2ee590afc26df355f3fb87db1e9af6fb7f740298dbec72df7c2";
                24 => "0xfddb01400ce1b4119ddbac337182c61e09adb32ee040af815d10e2236196baf2";
                25 => "0x86df4719e1b22d3006da6b235286866f06bcdc1d711fd548a0cb59fa5798d980";
                26 => "0x1836c75efc1d4a3f0f54bf154f71bcb723aeb357363e0374ab428e993cd5d5b1";
                27 => "0xa44908b0cabec0ef9bcd2362c8b1b9c5615a1f15955174a3901d43c3158687a8";
                28 => "0x451a5043e968379117975425198af6071afa481f49967c7dff959205047d7a9a";
                29 => "0xf7206f9a32691b10d4b5b82bf54c8d57762142c7e2a32af920699a43ed172031";
                30 => "0xf97fe617bd6d2a78d364ce68f6b5a864878246c84dd87fddf1bc78fc50baceb7";
                31 => "0x616cfe9ccea956a0709082df9b3ad6595ce832c5f3819b69907cc9c7ee611da4";
                32 => "0x6b14529b1bcf7e64a89c20b33b87a9d86cd17195eaf5bb8aa98a83ef8634a06d";
            }
        }

        /// Rust's own integer types have their own impls, so they get their own
        /// check against the matching Solidity width.
        #[test]
        fn rust_integer_keys_match_solidity() {
            assert_eq!(
                (-1i8).compute_slot(U256::from(1)),
                expect("0xc39d774f18115b85b81494d65e588b565d73abc969333d1da7b0a0eb0729accd")
            );
            assert_eq!(
                i8::MIN.compute_slot(U256::from(1)),
                expect("0xa6448894d065a3e7161c6293c2db5e883893c02ad215165c869901858b9036a9")
            );
            assert_eq!(
                1i8.compute_slot(U256::from(1)),
                expect("0xcc69885fda6bcc1a4ace058b4a62bf5e179ea78fd58a1ccd71c22cc9b688792f")
            );
            assert_eq!(
                (-1i16).compute_slot(U256::from(2)),
                expect("0x38b5b2ceac7637132d27514ffcf440b705287635075af7b8bd5adcaa6a4cc5bb")
            );
            assert_eq!(
                i16::MIN.compute_slot(U256::from(2)),
                expect("0xcf2fb756914b19221bf2fef06148e4b4b9392b9c9a1f0bc7d93709ef28342ba5")
            );
            assert_eq!(
                (-1i32).compute_slot(U256::from(4)),
                expect("0xd8c80a9840ed58f33f2186a8fbc29ecd8c3610d196f1da047301bd51988eb95c")
            );
            assert_eq!(
                i32::MIN.compute_slot(U256::from(4)),
                expect("0x6e55b48d4744edeccafa886884bc9a4dbd00a777c2dcd5791e8b167088622824")
            );
            assert_eq!(
                (-1i64).compute_slot(U256::from(8)),
                expect("0x50015d2c5500ee864adc0ae35838917a2d9a98eb2ab97342b4d689d9a074dbfa")
            );
            assert_eq!(
                i64::MIN.compute_slot(U256::from(8)),
                expect("0xb6f2a08fd84fd72638abdb192e8c8e97b6d535d5052c4a0421762dcd1a7379cb")
            );
            assert_eq!(
                (-1i128).compute_slot(U256::from(16)),
                expect("0x67c618532631f5e38a3b9c5e06a3e5553e5fac44409c6f6f7364f2525b56773a")
            );
            assert_eq!(
                i128::MIN.compute_slot(U256::from(16)),
                expect("0x5a065330214f8c8ac10a99d46ab28c8765aec8bb0237b9ab0d1086283ca05320")
            );

            assert_eq!(
                1u8.compute_slot(U256::from(1)),
                expect("0xcc69885fda6bcc1a4ace058b4a62bf5e179ea78fd58a1ccd71c22cc9b688792f")
            );
            assert_eq!(
                u8::MAX.compute_slot(U256::from(1)),
                expect("0x24a9e90595537a4321bf3a8fd43f02c179fe79a94dde54a8c1a057e2967a4d0b")
            );
            assert_eq!(
                1u16.compute_slot(U256::from(2)),
                expect("0xe90b7bceb6e7df5418fb78d8ee546e97c83a08bbccc01a0644d599ccd2a7c2e0")
            );
            assert_eq!(
                u16::MAX.compute_slot(U256::from(2)),
                expect("0x695395ec6a2c9d3a74f1c3d78bda956395489a2ec3ce495c3087446ede7bcc9d")
            );
            assert_eq!(
                1u32.compute_slot(U256::from(4)),
                expect("0xabd6e7cb50984ff9c2f3e18a2660c3353dadf4e3291deeb275dae2cd1e44fe05")
            );
            assert_eq!(
                u32::MAX.compute_slot(U256::from(4)),
                expect("0x2484d89f5223ef64143beaf0a743715076229ba3cba1b81333f195bbd499c493")
            );
            assert_eq!(
                1u64.compute_slot(U256::from(8)),
                expect("0xad67d757c34507f157cacfa2e3153e9f260a2244f30428821be7be64587ac55f")
            );
            assert_eq!(
                u64::MAX.compute_slot(U256::from(8)),
                expect("0x0b7244c513877c415f365e3266d1f5b7b85386b2438257e95327690b197b4523")
            );
            assert_eq!(
                1u128.compute_slot(U256::from(16)),
                expect("0x8c6065603763fec3f5742441d3833f3f43b982453612d76adb39a885e3006b5f")
            );
            assert_eq!(
                u128::MAX.compute_slot(U256::from(16)),
                expect("0x36da9b7b57d3afaa3e6cc456e64eaa514b2ae0d30afc92ea1ec06b30e88959ab")
            );
        }

        #[test]
        fn address_bool_and_dynamic_keys_match_solidity() {
            assert_eq!(
                Address::from(ADDR_A).compute_slot(U256::from(1)),
                expect("0x3c57502180841def3a766b77feb940197842ed9efd28b2f50e0f70458b82580a"),
                "address key",
            );
            assert_eq!(
                true.compute_slot(U256::from(2)),
                expect("0xe90b7bceb6e7df5418fb78d8ee546e97c83a08bbccc01a0644d599ccd2a7c2e0"),
                "bool key true",
            );
            assert_eq!(
                false.compute_slot(U256::from(2)),
                expect("0xac33ff75c19e70fe83507db0d683fd3465c996598dc972688b7ace676c89077b"),
                "bool key false",
            );
            assert_eq!(
                "alice".compute_slot(U256::from(5)),
                expect("0xfc294032e6b5f0d6e44152b2f364949f25109ae791ffea493e0e54b8b816e667"),
                "string key",
            );
            let bytes_key: &[u8] = &[0xde, 0xad, 0xbe, 0xef];
            assert_eq!(
                bytes_key.compute_slot(U256::from(6)),
                expect("0x815d41eb90fe7596bc0972a013d6cf8a713e439fd2fd2d44db1a23abf96a0321"),
                "bytes key",
            );
        }

        #[test]
        fn nested_map_slots_match_solidity() {
            // mapping(address => mapping(address => uint256)) at slot 3
            let nested = StorageMap::<Address, StorageMap<Address, StoragePrimitive<U256>>>::new(
                U256::from(3),
            );
            assert_eq!(
                nested
                    .entry(Address::from(ADDR_A))
                    .entry(Address::from(ADDR_B))
                    .slot(),
                expect("0xb3632e2932f8172291298022e8bb1f6e4c21b49aef132143d2bb7a105cd0eedc"),
            );

            // mapping(int24 => mapping(bytes4 => uint256)) at slot 4: both levels
            // pad the key differently from the packed storage layout.
            let mixed = StorageMap::<
                Signed<24, 1>,
                StorageMap<FixedBytes<4>, StoragePrimitive<U256>>,
            >::new(U256::from(4));
            assert_eq!(
                mixed
                    .entry(Signed::<24, 1>::MINUS_ONE)
                    .entry(FixedBytes::<4>::from([0xde, 0xad, 0xbe, 0xef]))
                    .slot(),
                expect("0x24e38c94117735d9899d5475e6d23fc3728dea0a5a9171209193b99a6074fb15"),
            );
        }

        /// `address` and `bytes20` carry the same 20 bytes but land on different
        /// slots in Solidity: one is right-aligned, the other left-aligned.
        #[test]
        fn address_and_bytes20_keys_do_not_collide() {
            let base = U256::from(1);
            let as_address = Address::from(ADDR_A).compute_slot(base);
            let as_bytes20 = FixedBytes::<20>::from(ADDR_A).compute_slot(base);

            assert_ne!(as_address, as_bytes20);
            assert_eq!(
                as_address,
                expect("0x3c57502180841def3a766b77feb940197842ed9efd28b2f50e0f70458b82580a")
            );
        }
    }

    #[test]
    fn test_map_zero_key() {
        let mut sdk = MockStorage::new();
        let map = StorageMap::<U256, StoragePrimitive<U256>>::new(U256::from(70));

        // Test with key = 0
        map.entry(U256::ZERO).set(&mut sdk, U256::from(0xABCDEF));

        // Calculate slot for key = 0
        let mut data = [0u8; 64];
        data[0..32].copy_from_slice(&U256::ZERO.to_be_bytes::<32>());
        data[32..64].copy_from_slice(&U256::from(70).to_be_bytes::<32>());
        let slot = U256::from_be_bytes(crypto_keccak256(data).0);

        assert_eq!(sdk.get_slot(slot), U256::from(0xABCDEF));
    }
}
