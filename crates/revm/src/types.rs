use crate::gas::Gas;
use fluentbase_types::ExitCode;
use revm_primitives::{Address, Bytes};

pub(crate) struct CallCreateResult {
    pub(crate) result: ExitCode,
    pub(crate) created_address: Option<Address>,
    pub(crate) gas: Gas,
    pub(crate) return_value: Bytes,
}

impl CallCreateResult {
    pub(crate) fn from_error(result: ExitCode, gas: Gas) -> Self {
        Self {
            result,
            created_address: None,
            gas,
            return_value: Bytes::new(),
        }
    }
}

enum BytecodeType {
    Rwasm,
    Evm,
    Wasm,
}

impl BytecodeType {
    pub(crate) fn from_slice(input: &[u8]) -> Self {
        if input.len() >= 4 && input[0..4] == [0x00, 0x61, 0x73, 0x6d] {
            Self::Wasm
        } else if input.len() >= 1 && input[0] == 0xef {
            Self::Rwasm
        } else {
            Self::Evm
        }
    }
}
