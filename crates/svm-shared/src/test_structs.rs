use alloc::{string::String, vec::Vec};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct ModifyAccount1 {
    pub account_idx: usize,
    pub byte_n_to_set: u32,
    pub byte_n_val: u8,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CreateAccountAndModifySomeData1 {
    pub lamports_to_send: u64,
    pub space: u32,
    pub seeds: Vec<Vec<u8>>,
    pub byte_n_to_set: u32,
    pub byte_n_value: u8,
}

// sol_big_mod_exp
#[derive(Clone, Serialize, Deserialize)]
pub struct SolBigModExp {
    pub base: String,
    pub exponent: String,
    pub modulus: String,
    pub expected: String,
}

impl SolBigModExp {
    pub fn new(base: &str, exponent: &str, modulus: &str, expected: &str) -> Self {
        Self {
            base: base.into(),
            exponent: exponent.into(),
            modulus: modulus.into(),
            expected: expected.into(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SolSecp256k1Recover {
    pub message: Vec<u8>,
    pub signature_bytes: Vec<u8>,
    pub recovery_id: u8,
    pub pubkey_bytes: Vec<u8>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Keccak256 {
    pub data: Vec<Vec<u8>>,
    pub expected_result: Vec<u8>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Sha256 {
    pub data: Vec<Vec<u8>>,
    pub expected_result: Vec<u8>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Blake3 {
    pub data: Vec<Vec<u8>>,
    pub expected_result: Vec<u8>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct SetGetReturnData {
    pub data: Vec<u8>,
}

#[derive(Clone, Serialize, Deserialize)]
pub enum TestCommand {
    ModifyAccount1(ModifyAccount1),
    CreateAccountAndModifySomeData1(CreateAccountAndModifySomeData1),
    SolBigModExp(SolBigModExp),
    SolSecp256k1Recover(SolSecp256k1Recover),
    Keccak256(Keccak256),
    Sha256(Sha256),
    Blake3(Blake3),
    SetGetReturnData(SetGetReturnData),
}

macro_rules! impl_from {
    ($typ:ident) => {
        impl From<$typ> for TestCommand {
            fn from(value: $typ) -> Self {
                TestCommand::$typ(value)
            }
        }
    };
}

impl_from!(ModifyAccount1);
impl_from!(CreateAccountAndModifySomeData1);
impl_from!(SolBigModExp);
impl_from!(SolSecp256k1Recover);
impl_from!(Keccak256);
impl_from!(Sha256);
impl_from!(Blake3);
impl_from!(SetGetReturnData);
