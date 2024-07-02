use alloy_primitives::Uint;
use core::{borrow::Borrow, fmt::Debug};
use fluentbase_sdk::{
    codec::{BufferDecoder, Encoder},
    contracts::{EvmAPI, EvmSloadInput, EvmSstoreInput},
    Address,
    Bytes,
    U256,
};

pub trait StorageValue<Client: EvmAPI, V: Encoder<V>> {
    fn get(&self, client: &Client, key: U256) -> Result<V, String>;
    fn set(&self, client: &Client, key: U256, value: V) -> Result<(), String>;
}

impl<Client: EvmAPI> StorageValue<Client, Address> for Address {
    fn get(&self, client: &Client, key: U256) -> Result<Address, String> {
        let input = EvmSloadInput { index: key };
        let output = client.sload(input);
        let chunk = output.value.to_be_bytes::<32>();

        let mut decoder = BufferDecoder::new(&chunk);
        let mut body = <Address>::default();
        <Address as Encoder<Address>>::decode_body(&mut decoder, 0, &mut body);
        Ok(body)
    }
    fn set(&self, client: &Client, key: U256, value: Address) -> Result<(), String> {
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
            let input = EvmSstoreInput {
                index: key + U256::from(i),
                value: value_u256,
            };
            client.sstore(input);
        }
        Ok(())
    }
}

impl<const BITS: usize, const LIMBS: usize, Client: EvmAPI> StorageValue<Client, Uint<BITS, LIMBS>>
    for Uint<BITS, LIMBS>
{
    fn get(&self, client: &Client, key: U256) -> Result<Uint<BITS, LIMBS>, String> {
        let input = EvmSloadInput { index: key };
        let output = client.sload(input);
        let chunk = output.value.to_be_bytes::<32>();

        let mut decoder = BufferDecoder::new(&chunk);
        let mut body = Uint::<BITS, LIMBS>::default();
        Uint::<BITS, LIMBS>::decode_body(&mut decoder, 0, &mut body);

        Ok(body)
    }

    fn set(&self, client: &Client, key: U256, value: Uint<BITS, LIMBS>) -> Result<(), String> {
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
            let input = EvmSstoreInput {
                index: key + U256::from(i),
                value: value_u256,
            };

            client.sstore(input);
        }

        Ok(())
    }
}

impl<Client: EvmAPI> StorageValue<Client, Bytes> for Bytes {
    fn get(&self, client: &Client, key: U256) -> Result<Bytes, String> {
        let output = client.sload(EvmSloadInput { index: key });
        let header_chunk = output.value.to_be_bytes::<32>();
        let mut decoder = BufferDecoder::new(&header_chunk);
        let (header_offset, data_len) =
            Bytes::decode_header(&mut decoder, 0, &mut Bytes::default());
        let chunk_size = 32;
        let num_chunks = (data_len + chunk_size - 1) / chunk_size;
        let mut buffer = Vec::with_capacity(num_chunks * chunk_size);
        for i in 0..num_chunks {
            let input = EvmSloadInput {
                index: key + U256::from(i + (header_offset / chunk_size)),
            };
            let output = client.sload(input);
            let chunk = output.value.to_be_bytes::<32>();
            buffer.extend_from_slice(&chunk);
        }
        buffer.truncate(header_offset + data_len);
        let mut decoder = BufferDecoder::new(&buffer);
        let mut body = Bytes::default();
        Bytes::decode_body(&mut decoder, 0, &mut body);
        Ok(body)
    }
    fn set(&self, client: &Client, key: U256, value: Bytes) -> Result<(), String> {
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
            let input = EvmSstoreInput {
                index: key + U256::from(i),
                value: value_u256,
            };

            client.sstore(input);
        }

        Ok(())
    }
}
