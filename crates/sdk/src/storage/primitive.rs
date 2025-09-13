use crate::storage::{
    PrimitiveAccess, PrimitiveCodec, StorageDescriptor, StorageLayout, StorageOps,
};
use core::marker::PhantomData;
use fluentbase_types::{Address, FixedBytes, Signed, StorageAPI, Uint, U256};

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

// Macro for alloy_primitives::Uint types
macro_rules! impl_uint_codec {
    ($($bits:literal => $limbs:literal),* $(,)?) => {
        $(
            impl PrimitiveCodec for Uint<$bits, $limbs> {
                const ENCODED_SIZE: usize = $bits / 8;

                fn encode_into(&self, target: &mut [u8]) {
                    debug_assert_eq!(target.len(), Self::ENCODED_SIZE);
                    let bytes = self.to_be_bytes::<{ $bits / 8 }>();
                    target.copy_from_slice(&bytes);
                }

                fn decode(bytes: &[u8]) -> Self {
                    debug_assert_eq!(bytes.len(), Self::ENCODED_SIZE);
                    Self::from_be_bytes::<{ $bits / 8 }>(bytes.try_into().unwrap())
                }
            }
        )*
    };
}

// Macro for alloy_primitives::Signed types
macro_rules! impl_signed_codec {
    ($($bits:literal => $limbs:literal),* $(,)?) => {
        $(
            impl PrimitiveCodec for Signed<$bits, $limbs> {
                const ENCODED_SIZE: usize = $bits / 8;

                fn encode_into(&self, target: &mut [u8]) {
                    debug_assert_eq!(target.len(), Self::ENCODED_SIZE);
                    let bytes = self.to_be_bytes::<{ $bits / 8 }>();
                    target.copy_from_slice(&bytes);
                }

                fn decode(bytes: &[u8]) -> Self {
                    debug_assert_eq!(bytes.len(), Self::ENCODED_SIZE);
                    Self::from_be_bytes::<{ $bits / 8 }>(bytes.try_into().unwrap())
                }
            }
        )*
    };
}

// Macro for Rust unsigned integer types (excluding those already implemented)
macro_rules! impl_rust_uint_codec {
    ($($ty:ty),* $(,)?) => {
        $(
            impl PrimitiveCodec for $ty {
                const ENCODED_SIZE: usize = core::mem::size_of::<$ty>();

                fn encode_into(&self, target: &mut [u8]) {
                    debug_assert_eq!(target.len(), Self::ENCODED_SIZE);
                    target.copy_from_slice(&self.to_be_bytes());
                }

                fn decode(bytes: &[u8]) -> Self {
                    debug_assert_eq!(bytes.len(), Self::ENCODED_SIZE);
                    <$ty>::from_be_bytes(bytes.try_into().unwrap())
                }
            }
        )*
    };
}

// Macro for Rust signed integer types
macro_rules! impl_rust_sint_codec {
    ($($ty:ty),* $(,)?) => {
        $(
            impl PrimitiveCodec for $ty {
                const ENCODED_SIZE: usize = core::mem::size_of::<$ty>();

                fn encode_into(&self, target: &mut [u8]) {
                    debug_assert_eq!(target.len(), Self::ENCODED_SIZE);
                    target.copy_from_slice(&self.to_be_bytes());
                }

                fn decode(bytes: &[u8]) -> Self {
                    debug_assert_eq!(bytes.len(), Self::ENCODED_SIZE);
                    <$ty>::from_be_bytes(bytes.try_into().unwrap())
                }
            }
        )*
    };
}

// Macro for alloy_primitives::FixedBytes types
macro_rules! impl_fixed_bytes_codec {
    ($($bytes:literal),* $(,)?) => {
        $(
            impl PrimitiveCodec for FixedBytes<$bytes> {
                const ENCODED_SIZE: usize = $bytes;

                fn encode_into(&self, target: &mut [u8]) {
                    debug_assert_eq!(target.len(), Self::ENCODED_SIZE);
                    target.copy_from_slice(self.as_slice());
                }

                fn decode(bytes: &[u8]) -> Self {
                    debug_assert_eq!(bytes.len(), Self::ENCODED_SIZE);
                    FixedBytes::from_slice(bytes)
                }
            }
        )*
    };
}

// Apply macros for all Uint types (LIMBS = (BITS + 63) / 64)
impl_uint_codec! {
    8 => 1,
    16 => 1,
    24 => 1,
    32 => 1,
    40 => 1,
    48 => 1,
    56 => 1,
    64 => 1,
    72 => 2,
    80 => 2,
    88 => 2,
    96 => 2,
    104 => 2,
    112 => 2,
    120 => 2,
    128 => 2,
    136 => 3,
    144 => 3,
    152 => 3,
    160 => 3,
    168 => 3,
    176 => 3,
    184 => 3,
    192 => 3,
    200 => 4,
    208 => 4,
    216 => 4,
    224 => 4,
    232 => 4,
    240 => 4,
    248 => 4,
    256 => 4,
}

// Apply macros for all Signed types
impl_signed_codec! {
    8 => 1,
    16 => 1,
    24 => 1,
    32 => 1,
    40 => 1,
    48 => 1,
    56 => 1,
    64 => 1,
    72 => 2,
    80 => 2,
    88 => 2,
    96 => 2,
    104 => 2,
    112 => 2,
    120 => 2,
    128 => 2,
    136 => 3,
    144 => 3,
    152 => 3,
    160 => 3,
    168 => 3,
    176 => 3,
    184 => 3,
    192 => 3,
    200 => 4,
    208 => 4,
    216 => 4,
    224 => 4,
    232 => 4,
    240 => 4,
    248 => 4,
    256 => 4,
}

// Apply macros for Rust primitive types
impl_rust_uint_codec! {
    u8, u16, u32, u64, u128, usize
}

impl_rust_sint_codec! {
    i8, i16, i32, i64, i128, isize
}

impl_fixed_bytes_codec! {
    1, 2, 3, 4, 5, 6, 7, 8,
    9, 10, 11, 12, 13, 14, 15, 16,
    17, 18, 19, 20, 21, 22, 23, 24,
    25, 26, 27, 28, 29, 30, 31, 32,
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

    #[test]
    fn test_uint_types() {
        let mut sdk = MockStorage::new();

        // Test U8 (Uint<8, 1>)
        let u8_accessor = <StoragePrimitive<Uint<8, 1>> as StorageLayout>::access(
            StoragePrimitive::<Uint<8, 1>>::new(U256::from(10), 31),
        );
        u8_accessor.set(&mut sdk, Uint::<8, 1>::from(255));
        assert_eq!(u8_accessor.get(&sdk), Uint::<8, 1>::from(255));

        // Test U128 (Uint<128, 2>)
        let u128_accessor =
            <StoragePrimitive<Uint<128, 2>> as StorageLayout>::access(StoragePrimitive::<
                Uint<128, 2>,
            >::new(
                U256::from(11), 16
            ));
        let u128_val = Uint::<128, 2>::from(0x12345678_9ABCDEF0_u128);
        u128_accessor.set(&mut sdk, u128_val);
        assert_eq!(u128_accessor.get(&sdk), u128_val);
    }

    #[test]
    fn test_signed_types() {
        let mut sdk = MockStorage::new();

        // Test I8 (Signed<8, 1>)
        let i8_accessor =
            <StoragePrimitive<Signed<8, 1>> as StorageLayout>::access(StoragePrimitive::<
                Signed<8, 1>,
            >::new(
                U256::from(20), 31
            ));
        i8_accessor.set(&mut sdk, Signed::<8, 1>::try_from(-42).unwrap());
        assert_eq!(
            i8_accessor.get(&sdk),
            Signed::<8, 1>::try_from(-42).unwrap()
        );

        // Test I256 with negative value
        let i256_accessor =
            <StoragePrimitive<Signed<256, 4>> as StorageLayout>::access(StoragePrimitive::<
                Signed<256, 4>,
            >::new(
                U256::from(21), 0
            ));
        let negative_val = Signed::<256, 4>::try_from(-1000000).unwrap();
        i256_accessor.set(&mut sdk, negative_val);
        assert_eq!(i256_accessor.get(&sdk), negative_val);
    }

    #[test]
    fn test_fixed_bytes_types() {
        let mut sdk = MockStorage::new();

        // Test FixedBytes<1> (bytes1)
        let bytes1_accessor =
            <StoragePrimitive<FixedBytes<1>> as StorageLayout>::access(StoragePrimitive::<
                FixedBytes<1>,
            >::new(
                U256::from(30), 31
            ));
        let bytes1_val = FixedBytes::<1>::from([0xAB]);
        bytes1_accessor.set(&mut sdk, bytes1_val);
        assert_eq!(bytes1_accessor.get(&sdk), bytes1_val);

        // Test FixedBytes<20> (bytes20 - address size)
        let bytes20_accessor =
            <StoragePrimitive<FixedBytes<20>> as StorageLayout>::access(StoragePrimitive::<
                FixedBytes<20>,
            >::new(
                U256::from(31), 12
            ));
        let bytes20_val = FixedBytes::<20>::from([0x42; 20]);
        bytes20_accessor.set(&mut sdk, bytes20_val);
        assert_eq!(bytes20_accessor.get(&sdk), bytes20_val);

        // Test FixedBytes<32> (bytes32 - full slot)
        let bytes32_accessor =
            <StoragePrimitive<FixedBytes<32>> as StorageLayout>::access(StoragePrimitive::<
                FixedBytes<32>,
            >::new(
                U256::from(32), 0
            ));
        let bytes32_val = FixedBytes::<32>::from([0xFF; 32]);
        bytes32_accessor.set(&mut sdk, bytes32_val);
        assert_eq!(bytes32_accessor.get(&sdk), bytes32_val);
    }

    #[test]
    fn test_rust_primitive_signed() {
        let mut sdk = MockStorage::new();

        // Test i8
        let i8_accessor = <StoragePrimitive<i8> as StorageLayout>::access(
            StoragePrimitive::<i8>::new(U256::from(40), 31),
        );
        i8_accessor.set(&mut sdk, -128i8);
        assert_eq!(i8_accessor.get(&sdk), -128i8);

        // Test i64
        let i64_accessor = <StoragePrimitive<i64> as StorageLayout>::access(
            StoragePrimitive::<i64>::new(U256::from(41), 24),
        );
        i64_accessor.set(&mut sdk, -9223372036854775808i64); // i64::MIN
        assert_eq!(i64_accessor.get(&sdk), -9223372036854775808i64);
    }

    #[test]
    fn test_rust_primitive_unsigned() {
        let mut sdk = MockStorage::new();

        // Test u128
        let u128_accessor = <StoragePrimitive<u128> as StorageLayout>::access(StoragePrimitive::<
            u128,
        >::new(
            U256::from(50), 16
        ));
        u128_accessor.set(&mut sdk, 340282366920938463463374607431768211455u128); // u128::MAX
        assert_eq!(
            u128_accessor.get(&sdk),
            340282366920938463463374607431768211455u128
        );
    }

    #[test]
    fn test_mixed_packing() {
        let mut sdk = MockStorage::new();
        let slot = U256::from(60);

        // Pack: I8 | U16 | FixedBytes<4> | bool
        // Offsets: 31 | 29 | 25 | 24

        let i8_accessor =
            <StoragePrimitive<Signed<8, 1>> as StorageLayout>::access(StoragePrimitive::<
                Signed<8, 1>,
            >::new(slot, 31));
        let u16_accessor = <StoragePrimitive<Uint<16, 1>> as StorageLayout>::access(
            StoragePrimitive::<Uint<16, 1>>::new(slot, 29),
        );
        let bytes4_accessor =
            <StoragePrimitive<FixedBytes<4>> as StorageLayout>::access(StoragePrimitive::<
                FixedBytes<4>,
            >::new(slot, 25));
        let bool_accessor =
            <StoragePrimitive<bool> as StorageLayout>::access(StoragePrimitive::<bool>::new(
                slot, 24,
            ));

        // Set values
        i8_accessor.set(&mut sdk, Signed::<8, 1>::try_from(-1).unwrap());
        u16_accessor.set(&mut sdk, Uint::<16, 1>::from(0xBEEF));
        bytes4_accessor.set(&mut sdk, FixedBytes::<4>::from([0xDE, 0xAD, 0xBE, 0xEF]));
        bool_accessor.set(&mut sdk, true);

        // Verify all values remain correct
        assert_eq!(i8_accessor.get(&sdk), Signed::<8, 1>::try_from(-1).unwrap());
        assert_eq!(u16_accessor.get(&sdk), Uint::<16, 1>::from(0xBEEF));
        assert_eq!(
            bytes4_accessor.get(&sdk),
            FixedBytes::<4>::from([0xDE, 0xAD, 0xBE, 0xEF])
        );
        assert!(bool_accessor.get(&sdk));
    }

    #[test]
    fn test_edge_cases() {
        let mut sdk = MockStorage::new();

        // Test signed MIN and MAX values
        let i128_accessor =
            <StoragePrimitive<Signed<128, 2>> as StorageLayout>::access(StoragePrimitive::<
                Signed<128, 2>,
            >::new(
                U256::from(70), 16
            ));

        // Test MIN
        let min_val = Signed::<128, 2>::MIN;
        i128_accessor.set(&mut sdk, min_val);
        assert_eq!(i128_accessor.get(&sdk), min_val);

        // Test MAX
        let max_val = Signed::<128, 2>::MAX;
        i128_accessor.set(&mut sdk, max_val);
        assert_eq!(i128_accessor.get(&sdk), max_val);

        // Test -1 (all bits set)
        let neg_one = Signed::<128, 2>::try_from(-1).unwrap();
        i128_accessor.set(&mut sdk, neg_one);
        assert_eq!(i128_accessor.get(&sdk), neg_one);
    }
}
