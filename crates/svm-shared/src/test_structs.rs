use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

pub const EXPECTED_RET_OK: u64 = 0;
pub const EXPECTED_RET_ERR: u64 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModifyAccount1 {
    pub account_idx: usize,
    pub byte_n_to_set: u32,
    pub byte_n_val: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateAccountAndModifySomeData1 {
    pub lamports_to_send: u64,
    pub space: u32,
    pub seeds: Vec<Vec<u8>>,
    pub byte_n_to_set: u32,
    pub byte_n_value: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Transfer {
    pub lamports: u64,
    pub seeds: Vec<Vec<u8>>,
}

type Address = [u8; 20];
type U256 = [u8; 32];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvmCall {
    pub address: [u8; 20],
    pub value: [u8; 32],
    pub gas_limit: u64,
    pub data: Vec<u8>,
    pub result_data_expected: Vec<u8>,
}

impl EvmCall {
    pub fn to_vec(&self) -> Vec<u8> {
        use core::mem::size_of;
        let mut result = Vec::with_capacity(
            size_of::<Address>() + size_of::<U256>() + size_of::<u64>() + self.data.len(),
        );

        result.extend_from_slice(&self.address);
        result.extend_from_slice(&self.value);
        result.extend_from_slice(&self.gas_limit.to_le_bytes());
        result.extend_from_slice(&self.data);

        result
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SolBigModExp {
    pub base: Vec<u8>,
    pub exponent: Vec<u8>,
    pub modulus: Vec<u8>,
    pub expected: Vec<u8>,
    pub expected_ret: u64,
}
impl SolBigModExp {
    pub fn from_hex(
        base_hex: &str,
        exponent_hex: &str,
        modulus_hex: &str,
        expected_hex: &str,
        expected_ret: u64,
    ) -> Self {
        Self {
            base: hex::decode(base_hex).unwrap(),
            exponent: hex::decode(exponent_hex).unwrap(),
            modulus: hex::decode(modulus_hex).unwrap(),
            expected: hex::decode(expected_hex).unwrap(),
            expected_ret,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SolSecp256k1Recover {
    pub message: Vec<u8>,
    pub signature_bytes: Vec<u8>,
    pub recovery_id: u8,
    pub pubkey_bytes: Vec<u8>,
    pub expected_ret: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Keccak256 {
    pub data: Vec<Vec<u8>>,
    pub expected_result: [u8; 32],
    pub expected_ret: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sha256 {
    pub data: Vec<Vec<u8>>,
    pub expected_result: [u8; 32],
    pub expected_ret: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Blake3 {
    pub data: Vec<Vec<u8>>,
    pub expected_result: [u8; 32],
    pub expected_ret: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Poseidon {
    pub parameters: u64,
    pub endianness: u64,
    pub data: Vec<Vec<u8>>,
    pub expected_result: [u8; 32],
    pub expected_ret: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SetGetReturnData {
    pub data: Vec<u8>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CurvePointValidation {
    pub curve_id: u64,
    pub point: [u8; 32],
    pub expected_ret: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CurveGroupOp {
    pub curve_id: u64,
    pub group_op: u64,
    pub left_input: [u8; 32],
    pub right_input: [u8; 32],
    pub expected_point: [u8; 32],
    pub expected_ret: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CurveMultiscalarMultiplication {
    pub curve_id: u64,
    pub scalars: Vec<[u8; 32]>,
    pub points: Vec<[u8; 32]>,
    pub expected_point: [u8; 32],
    pub expected_ret: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyscallAltBn128 {
    pub group_op: u64,
    pub input: Vec<u8>,
    pub expected_result: Vec<u8>,
    pub expected_ret: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AltBn128Compression {
    pub group_op: u64,
    pub input: Vec<u8>,
    pub expected_result: Vec<u8>,
    pub expected_ret: u64,
}

macro_rules! impl_from {
    ($typ_and_enum_branch:ident) => {
        impl From<$typ_and_enum_branch> for TestCommand {
            fn from(value: $typ_and_enum_branch) -> Self {
                TestCommand::$typ_and_enum_branch(value)
            }
        }
    };
    ($typ:ident, $enum_branch:ident) => {
        impl From<$typ> for TestCommand {
            fn from(value: $typ) -> Self {
                TestCommand::$enum_branch(value)
            }
        }
    };
}

macro_rules! impl_structs {
    ($($typ:ident),+ $(,)?) => {
        #[derive(Clone, Debug, Serialize, Deserialize)]
        pub enum TestCommand {
            $(
                $typ($typ),
            )+
        }
        $(
            impl_from!($typ);
        )+
    };
}

impl_structs!(
    ModifyAccount1,
    CreateAccountAndModifySomeData1,
    Transfer,
    EvmCall,
    SolBigModExp,
    SolSecp256k1Recover,
    Keccak256,
    Sha256,
    Blake3,
    Poseidon,
    SetGetReturnData,
    CurvePointValidation,
    CurveGroupOp,
    CurveMultiscalarMultiplication,
    SyscallAltBn128,
    AltBn128Compression,
);
