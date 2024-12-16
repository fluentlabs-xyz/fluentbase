use fluentbase_codec::{bytes::BytesMut, FluentABI, FluentEncoder, SolidityABI, SolidityEncoder};
use fluentbase_types::{SharedAPI, U256};

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
            let chunk = value.0.to_be_bytes::<32>();

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
            let chunk = value.0.to_be_bytes::<32>();

            if i * 32 > header_size && chunk.iter().all(|&x| x == 0) {
                break;
            }
            buf.extend_from_slice(&chunk);
        }
        FluentABI::<T>::decode(&buf, 0).unwrap_or_else(|_| T::default())
    }

    fn set(sdk: &mut SDK, key: U256, value: T) {
        let buffer_size = value.size_hint();
        let mut encoded_buffer = BytesMut::with_capacity(buffer_size);
        FluentABI::<T>::encode(&value, &mut encoded_buffer, 0).expect("Encoding failed");

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
