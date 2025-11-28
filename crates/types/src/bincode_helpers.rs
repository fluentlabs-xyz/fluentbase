use alloc::vec::Vec;
use bincode::config::{Configuration, Fixint, LittleEndian};

pub static BINCODE_CONFIG_DEFAULT: Configuration<LittleEndian, Fixint> = bincode::config::legacy();

pub fn encode<T: bincode::enc::Encode>(entity: &T) -> Result<Vec<u8>, bincode::error::EncodeError> {
    bincode::encode_to_vec(entity, BINCODE_CONFIG_DEFAULT.clone())
}

pub fn decode<T: bincode::de::Decode<()>>(
    src: &[u8],
) -> Result<(T, usize), bincode::error::DecodeError> {
    bincode::decode_from_slice(src, BINCODE_CONFIG_DEFAULT.clone())
}
