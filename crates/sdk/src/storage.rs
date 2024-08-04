use alloc::vec;
use fluentbase_codec::{BufferDecoder, Encoder};
use fluentbase_types::{SharedAPI, U256};

pub trait StorageValue<SDK: SharedAPI, T> {
    fn get(sdk: &SDK, key: U256) -> T;
    fn set(sdk: &mut SDK, key: U256, value: T);
}

impl<SDK: SharedAPI, T> StorageValue<SDK, T> for T
where
    T: Encoder<T> + Default,
{
    fn get(sdk: &SDK, key: U256) -> T {
        let header_size = T::HEADER_SIZE;

        let mut buffer = vec![];

        for i in 0.. {
            let key = key + U256::from(i);
            let value = sdk.storage(key);
            let chunk = value.to_be_bytes::<32>();
            if i * 32 > header_size && chunk.iter().all(|&x| x == 0) {
                break;
            }
            buffer.extend_from_slice(&chunk);
        }

        let mut decoder = BufferDecoder::new(&buffer);
        let mut body = T::default();
        T::decode_body(&mut decoder, 0, &mut body);
        body
    }

    fn set(sdk: &mut SDK, key: U256, value: T) {
        let encoded_buffer = value.encode_to_vec(0);

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
