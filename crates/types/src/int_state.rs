use alloc::vec;
use alloc::vec::Vec;
use bincode::config::{Configuration, Fixint, LittleEndian};
use bincode::error;
use bincode::serde::Compat;
use serde::{Deserialize, Serialize};

pub const INT_PREFIX: &[u8] = b"int_prefix";

#[derive(Default, Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct IntInitState {
    pub input: Vec<u8>,
    pub bytecode_pc: usize,
}

#[derive(Default, Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct IntOutcomeState {
    pub output: Vec<u8>,
    // pub bytecode_pc: usize,
    pub exit_code: i32,
    pub fuel_consumed: u64,
    pub fuel_refunded: i64,
    // pub gas_spent: u64,
}

#[derive(Default, Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct IntState {
    pub syscall_params: Vec<u8>,
    pub init: IntInitState,
    pub outcome: IntOutcomeState,
}

const BINCODE_CONFIG_DEFAULT: Configuration<LittleEndian, Fixint> = bincode::config::legacy();

pub fn bincode_encode<T: serde::ser::Serialize>(obj: &T) -> Vec<u8> {
    let mut buf = vec![];
    buf.extend(bincode::encode_to_vec(Compat(obj), BINCODE_CONFIG_DEFAULT).unwrap());
    buf
}
pub fn bincode_encode_prefixed<T: serde::ser::Serialize>(prefix: &[u8], obj: &T) -> Vec<u8> {
    let mut buf = prefix.to_vec();
    buf.extend(bincode_encode(obj));
    buf
}

pub fn bincode_try_decode<T: serde::de::DeserializeOwned>(
    buf: &[u8],
) -> Result<T, error::DecodeError> {
    let bincode_config = bincode::config::legacy();
    let result: Compat<T> = bincode::decode_from_slice(buf, bincode_config)?.0;
    Ok(result.0)
}
pub fn bincode_try_decode_prefixed<T: serde::de::DeserializeOwned>(
    prefix: &[u8],
    buf: &[u8],
) -> Result<T, error::DecodeError> {
    let data;
    if !buf.starts_with(prefix) {
        return Err(error::DecodeError::Other("incorrect prefix"));
    }
    data = &buf[prefix.len()..];
    bincode_try_decode(data)
}

#[cfg(test)]
mod tests {
    use crate::int_state::{
        bincode_encode_prefixed, bincode_try_decode_prefixed, IntInitState, IntOutcomeState,
        IntState,
    };

    #[test]
    fn enc_dec_test() {
        let obj1 = IntState {
            syscall_params: vec![1, 2, 3],
            init: IntInitState {
                input: vec![4, 5, 6],
                bytecode_pc: 1,
            },
            outcome: IntOutcomeState {
                output: vec![7, 8, 9],
                exit_code: 4,
                fuel_consumed: 1,
                fuel_refunded: 2,
            },
        };
        let obj1_enc = bincode_encode_prefixed(&[], &obj1);
        let obj1_dec: IntState = bincode_try_decode_prefixed(&[], &obj1_enc).unwrap();
        assert_eq!(obj1, obj1_dec);
    }
}
