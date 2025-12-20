use crate::{
    storage::{PackableCodec, StorageDescriptor, StorageLayout, StorageOps},
    Address, FixedBytes, Signed, StorageAPI, Uint, U256,
};
use core::marker::PhantomData;
use fluentbase_types::ExitCode;

/// Storage descriptor and accessor for single packable values.
#[derive(Debug, PartialEq, Eq)]
pub struct StoragePrimitive<T> {
    slot: U256,
    offset: u8,
    _marker: PhantomData<T>,
}

// Manual Copy/Clone to avoid T: Copy bound.
// StoragePrimitive is just metadata (location), not the actual stored value.
impl<T> Clone for StoragePrimitive<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for StoragePrimitive<T> {}

impl<T> StorageDescriptor for StoragePrimitive<T> {
    fn new(slot: U256, offset: u8) -> Self {
        Self {
            slot,
            offset,
            _marker: PhantomData,
        }
    }

    fn slot(&self) -> U256 {
        self.slot
    }

    fn offset(&self) -> u8 {
        self.offset
    }
}

impl<T: PackableCodec> StoragePrimitive<T> {
    /// Read value from storage.
    pub fn get<S: StorageAPI>(&self, sdk: &S) -> T {
        sdk.read_at(self.slot, self.offset).unwrap()
    }

    pub fn get_checked<S: StorageAPI>(&self, sdk: &S) -> Result<T, ExitCode> {
        sdk.read_at(self.slot, self.offset)
    }

    /// Write value to storage.
    pub fn set<S: StorageAPI>(&self, sdk: &mut S, value: T) {
        sdk.write_at(self.slot, self.offset, &value).unwrap()
    }

    pub fn set_checked<S: StorageAPI>(&self, sdk: &mut S, value: T) -> Result<(), ExitCode> {
        sdk.write_at(self.slot, self.offset, &value)
    }
}

impl<T: PackableCodec + Copy> StorageLayout for StoragePrimitive<T> {
    type Descriptor = Self;
    type Accessor = Self;

    const BYTES: usize = T::ENCODED_SIZE;
    const SLOTS: usize = if T::ENCODED_SIZE == 32 { 1 } else { 0 };

    fn access(descriptor: Self::Descriptor) -> Self::Accessor {
        descriptor
    }
}

// --- PackableCodec implementations ---

impl PackableCodec for bool {
    const ENCODED_SIZE: usize = 1;

    fn encode_into(&self, target: &mut [u8]) {
        target[0] = *self as u8;
    }

    fn decode(bytes: &[u8]) -> Self {
        bytes[0] != 0
    }
}

impl PackableCodec for Address {
    const ENCODED_SIZE: usize = 20;

    fn encode_into(&self, target: &mut [u8]) {
        target.copy_from_slice(self.as_slice());
    }

    fn decode(bytes: &[u8]) -> Self {
        Address::from_slice(bytes)
    }
}

// Standard Rust integers
macro_rules! impl_int_codec {
    ($($ty:ty),*) => {
        $(
            impl PackableCodec for $ty {
                const ENCODED_SIZE: usize = core::mem::size_of::<$ty>();

                fn encode_into(&self, target: &mut [u8]) {
                    target.copy_from_slice(&self.to_be_bytes());
                }

                fn decode(bytes: &[u8]) -> Self {
                    <$ty>::from_be_bytes(bytes.try_into().unwrap())
                }
            }
        )*
    };
}

impl_int_codec!(u8, u16, u32, u64, u128, i8, i16, i32, i64, i128);

// Macro for Uint types (standard Solidity sizes)
macro_rules! impl_uint_codec {
    ($($bits:literal => $limbs:literal),*) => {
        $(
            impl PackableCodec for Uint<$bits, $limbs> {
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

// Macro for Signed types
macro_rules! impl_signed_codec {
    ($($bits:literal => $limbs:literal),*) => {
        $(
            impl PackableCodec for Signed<$bits, $limbs> {
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

// Macro for FixedBytes types
macro_rules! impl_fixed_bytes_codec {
    ($($n:literal),*) => {
        $(
            impl PackableCodec for FixedBytes<$n> {
                const ENCODED_SIZE: usize = $n;

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

// Standard Solidity uint types (LIMBS = (BITS + 63) / 64)
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
    256 => 4
}

// Standard Solidity int types
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
    256 => 4
}

// Standard Solidity bytes types
impl_fixed_bytes_codec! {
    1, 2, 3, 4, 5, 6, 7, 8,
    9, 10, 11, 12, 13, 14, 15, 16,
    17, 18, 19, 20, 21, 22, 23, 24,
    25, 26, 27, 28, 29, 30, 31, 32
}

macro_rules! storage_alias {
    ($($name:ident => $type:ty),* $(,)?) => {
        $(
            pub type $name = StoragePrimitive<$type>;
        )*
    };
}

storage_alias! {
    // Essential types
    StorageBool => bool,
    StorageAddress => Address,
    StorageU256 => U256,

    // Common Rust integers
    StorageU8 => u8,
    StorageU16 => u16,
    StorageU32 => u32,
    StorageU64 => u64,
    StorageU128 => u128,
    StorageI8 => i8,
    StorageI16 => i16,
    StorageI32 => i32,
    StorageI64 => i64,
    StorageI128 => i128,

    // Most used Solidity-compatible types
    StorageUint8 => Uint<8, 1>,
    StorageUint16 => Uint<16, 1>,
    StorageUint32 => Uint<32, 1>,
    StorageUint64 => Uint<64, 1>,
    StorageUint128 => Uint<128, 2>,
    StorageUint256 => Uint<256, 4>,

    StorageInt8 => Signed<8, 1>,
    StorageInt16 => Signed<16, 1>,
    StorageInt32 => Signed<32, 1>,
    StorageInt64 => Signed<64, 1>,
    StorageInt128 => Signed<128, 2>,
    StorageInt256 => Signed<256, 4>,

    StorageBytes4 => FixedBytes<4>,
    StorageBytes8 => FixedBytes<8>,
    StorageBytes16 => FixedBytes<16>,
    StorageBytes20 => FixedBytes<20>,
    StorageBytes32 => FixedBytes<32>,
    StorageB256 => StoragePrimitive<FixedBytes<32>>,
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::mock::MockStorage;

    #[test]
    fn test_packing_isolation() {
        // Critical: verify packed values don't overwrite each other
        let mut sdk = MockStorage::new();
        let slot = U256::from(0);

        // Pack: bool | u8 | u16 | u32
        // Offsets: 31 | 30 | 28 | 24
        let bool_val = StorageBool::new(slot, 31);
        let u8_val = StorageU8::new(slot, 30);
        let u16_val = StorageU16::new(slot, 28);
        let u32_val = StorageU32::new(slot, 24);

        // Set all values
        bool_val.set(&mut sdk, true);
        u8_val.set(&mut sdk, 0xFF);
        u16_val.set(&mut sdk, 0xABCD);
        u32_val.set(&mut sdk, 0x12345678);

        // Verify isolation - each value preserved
        assert!(bool_val.get(&sdk));
        assert_eq!(u8_val.get(&sdk), 0xFF);
        assert_eq!(u16_val.get(&sdk), 0xABCD);
        assert_eq!(u32_val.get(&sdk), 0x12345678);

        // Overwrite one value and check others unchanged
        u16_val.set(&mut sdk, 0x5678);
        assert!(bool_val.get(&sdk));
        assert_eq!(u8_val.get(&sdk), 0xFF);
        assert_eq!(u16_val.get(&sdk), 0x5678);
        assert_eq!(u32_val.get(&sdk), 0x12345678);
    }

    #[test]
    fn test_full_slot_optimization() {
        // Verify that full slot writes don't do unnecessary reads
        let mut sdk = MockStorage::new();

        // Pre-fill slot with garbage
        sdk.init_slot(
            U256::from(1),
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        );

        // Write full slot value - should overwrite without reading
        let value = StorageU256::new(U256::from(1), 0);
        value.set(&mut sdk, U256::from(42));

        // Verify complete overwrite
        assert_eq!(
            sdk.get_slot_hex(U256::from(1)),
            "000000000000000000000000000000000000000000000000000000000000002a"
        );
    }

    #[test]
    fn test_signed_encoding() {
        // Critical: verify two's complement encoding for negative values
        let mut sdk = MockStorage::new();

        let i8_val = StoragePrimitive::<i8>::new(U256::from(2), 31);
        let i128_val = StoragePrimitive::<i128>::new(U256::from(3), 16);

        // Test negative values
        i8_val.set(&mut sdk, -1);
        i128_val.set(&mut sdk, -42);

        assert_eq!(i8_val.get(&sdk), -1);
        assert_eq!(i128_val.get(&sdk), -42);

        // Verify encoding in storage (two's complement)
        let slot2 = sdk.get_slot_hex(U256::from(2));
        assert_eq!(&slot2[62..64], "ff"); // -1 as i8 = 0xFF
    }
}
