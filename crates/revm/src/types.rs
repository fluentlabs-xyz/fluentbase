use crate::gas::Gas;
use revm_primitives::{Address, Bytes};

pub(crate) struct CallCreateResult {
    pub(crate) result: i32,
    pub(crate) created_address: Option<Address>,
    pub(crate) gas: Gas,
    pub(crate) return_value: Bytes,
}

impl CallCreateResult {
    pub(crate) fn from_error<T: Into<i32>>(result: T, gas: Gas) -> Self {
        Self {
            result: result.into(),
            created_address: None,
            gas,
            return_value: Bytes::new(),
        }
    }
}

#[allow(non_camel_case_types)]
pub(crate) enum BytecodeType {
    EVM,
    WASM,
}

impl BytecodeType {
    pub(crate) fn from_slice(input: &[u8]) -> Self {
        if input.len() >= 4 && input[0..4] == [0x00, 0x61, 0x73, 0x6d] {
            Self::WASM
        } else {
            Self::EVM
        }
    }
}
