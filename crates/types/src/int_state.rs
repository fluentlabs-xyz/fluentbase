use crate::bytes::{Buf, BufMut};
use alloc::vec::Vec;
use bincode::error;
use bincode::serde::Compat;
use serde::{Deserialize, Serialize};

pub const INT_PREFIX: &[u8] = b"int_prefix";

#[derive(Default, Serialize, Deserialize, Debug, PartialEq)]
pub struct IntInitState {
    pub input: Vec<u8>,
    pub interpreter_stack: Vec<[u8; 32]>,
    pub bytecode_pc: usize,
}

#[derive(Default, Serialize, Deserialize, Debug, PartialEq)]
pub struct IntOutcomeState {
    pub output: Vec<u8>,
    pub interpreter_stack: Vec<[u8; 32]>,
    pub bytecode_pc: usize,
    pub exit_code: i32,
    pub gas_spent: u64,
}

#[derive(Default, Serialize, Deserialize, Debug, PartialEq)]
pub struct IntState {
    pub syscall_params: Vec<u8>,
    pub init: IntInitState,
    pub outcome: IntOutcomeState,
}

pub fn bincode_encode<T: serde::ser::Serialize>(prefix: &[u8], int_state: &T) -> Vec<u8> {
    let mut buf: Vec<u8> = prefix.to_vec();
    let bincode_config = bincode::config::legacy();
    buf.extend(bincode::encode_to_vec(Compat(int_state), bincode_config).unwrap());
    buf
}
pub fn bincode_try_decode<T: serde::de::DeserializeOwned>(
    prefix: &[u8],
    buf: &[u8],
) -> Result<T, error::DecodeError> {
    let data;
    if !buf.starts_with(prefix) {
        return Err(error::DecodeError::Other("incorrect prefix"));
    }
    data = &buf[prefix.len()..];
    let bincode_config = bincode::config::legacy();
    let result: Compat<T> = bincode::decode_from_slice(data, bincode_config)?.0;
    Ok(result.0)
}

#[cfg(test)]
mod tests {
    use crate::int_state::{
        bincode_encode, bincode_try_decode, IntInitState, IntOutcomeState, IntState,
    };

    #[test]
    fn enc_dec_test() {
        let obj1 = IntState {
            syscall_params: vec![1, 2, 3],
            init: IntInitState {
                input: vec![4, 5, 6],
                bytecode_pc: 1,
                interpreter_stack: vec![],
            },
            outcome: IntOutcomeState {
                output: vec![7, 8, 9],
                interpreter_stack: vec![],
                bytecode_pc: 12,
                exit_code: 4,
                gas_spent: 222,
            },
        };
        let obj1_enc = bincode_encode(&[], &obj1);
        let obj1_dec: IntState = bincode_try_decode(&[], &obj1_enc).unwrap();
        assert_eq!(obj1, obj1_dec);
    }
}
