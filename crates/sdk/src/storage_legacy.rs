use crate::{Address, FixedBytes, SharedAPI, I256, U256};
use fluentbase_codec::{bytes::BytesMut, CompactABI, FluentEncoder, SolidityABI, SolidityEncoder};

// StorageValueSolidity trait with Solidity-specific parameters
pub trait StorageValueSolidity<SDK: SharedAPI, T>
where
    T: SolidityEncoder + Default,
{
    fn get(sdk: &SDK, key: U256) -> T;
    fn set(sdk: &mut SDK, key: U256, value: T);
}

// StorageValueFluent trait with Fluent-specific parameters
pub trait StorageValueFluent<SDK: SharedAPI, T>
where
    T: FluentEncoder + Default,
{
    fn get(sdk: &SDK, key: U256) -> T;
    fn set(sdk: &mut SDK, key: U256, value: T);
}

// Implementation for Solidity mode (ALIGN = 32, SOL_MODE = true)
impl<SDK: SharedAPI, T> StorageValueSolidity<SDK, T> for T
where
    T: SolidityEncoder + Default,
{
    fn get(sdk: &SDK, key: U256) -> T {
        let header_size = T::SOLIDITY_HEADER_SIZE;
        let mut buf = BytesMut::new();

        for i in 0.. {
            let storage_key = key + U256::from(i);
            let value = sdk.storage(&storage_key);
            let chunk = value.data.to_be_bytes::<32>();

            if i * 32 > header_size && chunk.iter().all(|&x| x == 0) {
                break;
            }
            buf.extend_from_slice(&chunk);
        }
        SolidityABI::<T>::decode(&buf, 0).unwrap_or_else(|_| T::default())
    }

    fn set(sdk: &mut SDK, key: U256, value: T) {
        let buffer_size = value.size_hint();
        let mut encoded_buffer = BytesMut::with_capacity(buffer_size);
        SolidityABI::<T>::encode(&value, &mut encoded_buffer, 0).expect("Encoding failed");

        let chunk_size = 32;
        let num_chunks = (encoded_buffer.len() + chunk_size - 1) / chunk_size;

        for i in 0..num_chunks {
            let start = i * chunk_size;
            let end = (start + chunk_size).min(encoded_buffer.len());
            let chunk = &encoded_buffer[start..end];

            let mut chunk_padded = [0u8; 32];
            chunk_padded[..chunk.len()].copy_from_slice(chunk);

            let value_u256 = U256::from_be_bytes(chunk_padded);
            sdk.write_storage(key + U256::from(i), value_u256);
        }
    }
}

// Implementation for Fluent mode (ALIGN = 4, SOL_MODE = false)
impl<SDK: SharedAPI, T> StorageValueFluent<SDK, T> for T
where
    T: FluentEncoder + Default,
{
    fn get(sdk: &SDK, key: U256) -> T {
        let header_size = T::FLUENT_HEADER_SIZE;
        let mut buf = BytesMut::new();

        for i in 0.. {
            let storage_key = key + U256::from(i);
            let value = sdk.storage(&storage_key);
            let chunk = value.data.to_be_bytes::<32>();

            if i * 32 > header_size && chunk.iter().all(|&x| x == 0) {
                break;
            }
            buf.extend_from_slice(&chunk);
        }
        CompactABI::<T>::decode(&buf, 0).unwrap_or_else(|_| T::default())
    }

    fn set(sdk: &mut SDK, key: U256, value: T) {
        let buffer_size = value.size_hint();
        let mut encoded_buffer = BytesMut::with_capacity(buffer_size);
        CompactABI::<T>::encode(&value, &mut encoded_buffer, 0)
            .unwrap_or_else(|_| unreachable!("ABI encoding failure"));

        let chunk_size = 32;
        let num_chunks = (encoded_buffer.len() + chunk_size - 1) / chunk_size;

        for i in 0..num_chunks {
            let start = i * chunk_size;
            let end = (start + chunk_size).min(encoded_buffer.len());
            let chunk = &encoded_buffer[start..end];

            let mut chunk_padded = [0u8; 32];
            chunk_padded[..chunk.len()].copy_from_slice(chunk);

            let value_u256 = U256::from_be_bytes(chunk_padded);
            sdk.write_storage(key + U256::from(i), value_u256);
        }
    }
}

/// Trait for types that can be directly stored in blockchain storage
/// without requiring encoding/decoding.
pub trait DirectStorage<SDK: SharedAPI> {
    /// Get a value directly from storage
    fn get(sdk: &SDK, key: U256) -> Self;

    /// Set a value directly in storage
    fn set(sdk: &mut SDK, key: U256, value: Self);
}

impl<SDK: SharedAPI> DirectStorage<SDK> for U256 {
    fn get(sdk: &SDK, key: U256) -> Self {
        let value = sdk.storage(&key);
        // U256 is stored directly in the 32-byte slot
        value.data
    }

    fn set(sdk: &mut SDK, key: U256, value: Self) {
        sdk.write_storage(key, value);
    }
}

impl<SDK: SharedAPI> DirectStorage<SDK> for I256 {
    fn get(sdk: &SDK, key: U256) -> Self {
        let value = sdk.storage(&key);
        // Convert U256 to I256 - U256 is stored directly in the 32-byte slot
        I256::from_raw(value.data)
    }

    fn set(sdk: &mut SDK, key: U256, value: Self) {
        // Convert I256 to U256 for storage
        let u256_value = value.into_raw();
        sdk.write_storage(key, u256_value);
    }
}

// Implementation for Address (20 bytes)
impl<SDK: SharedAPI> DirectStorage<SDK> for Address {
    fn get(sdk: &SDK, key: U256) -> Self {
        let value = sdk.storage(&key);
        // Address is stored in the lower 20 bytes of the 32-byte slot
        let bytes = value.data.to_be_bytes::<32>();
        Address::from_slice(&bytes[12..32])
    }

    fn set(sdk: &mut SDK, key: U256, value: Self) {
        // Create a U256 with the address in the lower 20 bytes
        let mut bytes = [0u8; 32];
        bytes[12..32].copy_from_slice(&value[..]);
        let value_u256 = U256::from_be_bytes(bytes);
        sdk.write_storage(key, value_u256);
    }
}

// Implementation for bool
impl<SDK: SharedAPI> DirectStorage<SDK> for bool {
    fn get(sdk: &SDK, key: U256) -> Self {
        let value = sdk.storage(&key);
        // Boolean is stored as 0 or 1 in the lowest byte
        !value.data.is_zero()
    }

    fn set(sdk: &mut SDK, key: U256, value: Self) {
        let value_u256 = if value { U256::from(1) } else { U256::from(0) };
        sdk.write_storage(key, value_u256);
    }
}

impl<SDK: SharedAPI> DirectStorage<SDK> for u128 {
    fn get(sdk: &SDK, key: U256) -> Self {
        let value = sdk.storage(&key).data;
        let limbs = value.as_limbs();

        u128::from(limbs[0]) | (u128::from(limbs[1]) << 64)
    }

    fn set(sdk: &mut SDK, key: U256, value: Self) {
        // Write the FixedBytes directly to storage
        sdk.write_storage(key, U256::from(value));
    }
}

macro_rules! impl_direct_storage_for_uint {
    ($($t:ty),*) => {
        $(
            impl<SDK: SharedAPI> DirectStorage<SDK> for $t {
                fn get(sdk: &SDK, key: U256) -> Self {
                    let value = sdk.storage(&key).data;

                    // Extract from the U256 and cast to the target type
                    value.as_limbs()[0] as $t
                }

                fn set(sdk: &mut SDK, key: U256, value: Self) {
                    // Write the value to storage after converting to U256
                    sdk.write_storage(key, U256::from(value));
                }
            }
        )*
    };
}

impl_direct_storage_for_uint!(u8, u16, u32, u64);

impl<SDK: SharedAPI> DirectStorage<SDK> for i128 {
    fn get(sdk: &SDK, key: U256) -> Self {
        let value = sdk.storage(&key).data;
        let limbs = value.as_limbs();
        (u128::from(limbs[0]) | (u128::from(limbs[1]) << 64)) as i128
    }

    fn set(sdk: &mut SDK, key: U256, value: Self) {
        sdk.write_storage(key, U256::from(value as u128));
    }
}

macro_rules! impl_direct_storage_for_int {
    ($($t:ty),*) => {
        $(
            impl<SDK: SharedAPI> DirectStorage<SDK> for $t {
                fn get(sdk: &SDK, key: U256) -> Self {
                    let value = sdk.storage(&key).data;
                    value.as_limbs()[0] as $t
                }

                fn set(sdk: &mut SDK, key: U256, value: Self) {
                    sdk.write_storage(key, U256::from(value as u64));
                }
            }
        )*
    };
}

impl_direct_storage_for_int!(i8, i16, i32, i64);

// Macro to implement DirectStorage trait for both FixedBytes<N> and [u8; N] types
// where N is less than or equal to 32 bytes (the size of a storage slot)
macro_rules! impl_direct_storage_for_small_bytes {
    // For each size N from 0 to 32, generate implementations
    ($($n:expr),*) => {
        $(
            // Implementation for FixedBytes<N>
            // This allows FixedBytes of size N to be stored directly in a single storage slot
            impl<SDK: SharedAPI> DirectStorage<SDK> for FixedBytes<$n> {
                fn get(sdk: &SDK, key: U256) -> Self {
                    // Read the entire 32-byte slot from storage
                    let value = sdk.storage(&key);
                    let bytes = value.data.to_be_bytes::<32>();

                    // Copy only the first N bytes that we need
                    let mut result = [0u8; $n];
                    result.copy_from_slice(&bytes[0..$n]);

                    FixedBytes(result)
                }

                fn set(sdk: &mut SDK, key: U256, value: Self) {
                    // Create a new 32-byte array to represent the full storage slot
                    let mut bytes = [0u8; 32];

                    // Copy our N bytes into the beginning of the slot
                    bytes[0..$n].copy_from_slice(&value.0[..]);

                    // Convert to U256 and write to storage
                    let value_u256 = U256::from_be_bytes(bytes);
                    sdk.write_storage(key, value_u256);
                }
            }

            // Implementation for raw byte arrays [u8; N]
            // This provides the same functionality but for standard Rust arrays
            impl<SDK: SharedAPI> DirectStorage<SDK> for [u8; $n] {
                fn get(sdk: &SDK, key: U256) -> Self {
                    // Read the entire 32-byte slot from storage
                    let value = sdk.storage(&key);
                    let bytes = value.data.to_be_bytes::<32>();

                    // Copy only the first N bytes that we need
                    let mut result = [0u8; $n];
                    result.copy_from_slice(&bytes[0..$n]);

                    result
                }

                fn set(sdk: &mut SDK, key: U256, value: Self) {
                    // Create a new 32-byte array to represent the full storage slot
                    let mut bytes = [0u8; 32];

                    // Copy our N bytes into the beginning of the slot
                    bytes[0..$n].copy_from_slice(&value[..]);

                    // Convert to U256 and write to storage
                    let value_u256 = U256::from_be_bytes(bytes);
                    sdk.write_storage(key, value_u256);
                }
            }
        )*
    };
}

// Generate implementations for all valid sizes from 1 to 32 bytes
impl_direct_storage_for_small_bytes!(
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
    27, 28, 29, 30, 31, 32
);
