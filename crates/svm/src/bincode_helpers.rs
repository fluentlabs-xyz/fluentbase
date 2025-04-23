use bincode::{
    config,
    config::{Configuration, Fixint, LittleEndian},
    enc,
};

lazy_static::lazy_static! {
    pub static ref BINCODE_CONFIG_DEFAULT: Configuration<LittleEndian, Fixint> = config::legacy();
}

pub fn bincode_serialize_into_config<T: enc::Encode, C: config::Config>(
    entity: &T,
    dst: &mut [u8],
    config: C,
) -> Result<usize, bincode::error::EncodeError> {
    bincode::encode_into_slice(entity, dst, config)
}

pub fn bincode_serialize_into<T: enc::Encode>(
    entity: &T,
    dst: &mut [u8],
) -> Result<usize, bincode::error::EncodeError> {
    bincode_serialize_into_config(entity, dst, BINCODE_CONFIG_DEFAULT.clone())
}

pub fn bincode_serialize_original<T: enc::Encode>(
    entity: &T,
) -> Result<(Vec<u8>, usize), bincode::error::EncodeError> {
    let mut buf = vec![];
    let bytes_written = bincode_serialize_into(entity, &mut buf)?;
    Ok((buf, bytes_written))
}

pub fn bincode_serialize_config<T: enc::Encode, C: config::Config>(
    entity: &T,
    config: C,
) -> Result<Vec<u8>, bincode::error::EncodeError> {
    Ok(bincode::encode_to_vec(entity, config)?)
}

pub fn bincode_serialize<T: enc::Encode>(
    entity: &T,
) -> Result<Vec<u8>, bincode::error::EncodeError> {
    bincode_serialize_config(entity, BINCODE_CONFIG_DEFAULT.clone())
}

pub fn bincode_serialized_size<T: enc::Encode>(
    entity: &T,
) -> Result<usize, bincode::error::EncodeError> {
    // TODO need mor efficient way to extract serialized size
    let result = bincode_serialize(entity)?;
    Ok(result.len())
}

pub fn bincode_deserialize<'a, T: bincode::de::Decode<()>>(
    src: &[u8],
) -> Result<T, bincode::error::DecodeError> {
    Ok(bincode::decode_from_reader(src, BINCODE_CONFIG_DEFAULT.clone())?.0)
}
