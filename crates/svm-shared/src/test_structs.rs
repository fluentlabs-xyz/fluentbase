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

#[derive(Clone, Serialize, Deserialize)]
pub struct SolBigModExp {
    pub base: String,
    pub exponent: String,
    pub modulus: String,
    pub expected: String,
}

// sol_big_mod_exp
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
pub enum TestCommand {
    ModifyAccount1(ModifyAccount1),
    CreateAccountAndModifySomeData1(CreateAccountAndModifySomeData1),
    SolBigModExp(SolBigModExp),
}
