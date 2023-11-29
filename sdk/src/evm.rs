use crate::{SysPlatformSDK, SDK};
use alloc::{vec, vec::Vec};
use alloy_primitives::{Address, Bytes, B256, U256};
use alloy_rlp::{Decodable, Encodable, RlpDecodable, RlpEncodable};

#[derive(Default, Debug, Clone, RlpEncodable, RlpDecodable)]
pub struct ContractInput {
    pub input: Bytes,
    pub bytecode: Bytes,
    pub hash: B256,
    pub address: Address,
    pub caller: Address,
    pub value: U256,
}

#[derive(Debug, Copy, Clone)]
pub enum ContractError {
    InputDecodeFailure = -3001,
}

impl ContractInput {
    pub fn read_from_input() -> Self {
        const N_BUFFER: usize = 1024;
        let mut buffer = vec![0u8; N_BUFFER];
        let n = SDK::sys_read(buffer.as_mut_slice(), 0);
        if n as usize > N_BUFFER {
            buffer.resize(n as usize, 0u8);
            SDK::sys_read(&mut buffer.as_mut_slice()[0..N_BUFFER], N_BUFFER as u32);
        }
        Self::read_from_slice(buffer.as_slice())
    }

    pub fn read_from_slice(mut input: &[u8]) -> Self {
        let res = Self::decode(&mut input);
        if res.is_err() {
            SDK::sys_halt(ContractError::InputDecodeFailure as i32);
        }
        res.unwrap()
    }

    pub fn into_vec(self) -> Vec<u8> {
        let mut raw_contract_input: Vec<u8> = vec![];
        self.encode(&mut raw_contract_input);
        raw_contract_input
    }
}
