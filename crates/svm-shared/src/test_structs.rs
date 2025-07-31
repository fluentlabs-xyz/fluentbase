use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

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
pub struct SolSecp256k1RecoverOriginal {
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
pub struct Sha256Original {
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
impl From<Sha256Original> for Sha256 {
    fn from(value: Sha256Original) -> Self {
        Self {
            data: value.data,
            expected_result: value.expected_result,
            expected_ret: value.expected_ret,
        }
    }
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
pub struct CurvePointValidationOriginal {
    pub curve_id: u64,
    pub point: [u8; 32],
    pub expected_ret: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CurvePointValidation {
    pub curve_id: u64,
    pub point: [u8; 32],
    pub expected_ret: u64,
}
impl From<CurvePointValidationOriginal> for CurvePointValidation {
    fn from(value: CurvePointValidationOriginal) -> Self {
        Self {
            curve_id: value.curve_id,
            point: value.point,
            expected_ret: value.expected_ret,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CurveGroupOpOriginal {
    pub curve_id: u64,
    pub group_op: u64,
    pub left_input: [u8; 32],
    pub right_input: [u8; 32],
    pub expected_point: [u8; 32],
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
impl From<CurveGroupOpOriginal> for CurveGroupOp {
    fn from(value: CurveGroupOpOriginal) -> Self {
        Self {
            curve_id: value.curve_id,
            group_op: value.group_op,
            left_input: value.left_input,
            right_input: value.right_input,
            expected_point: value.expected_point,
            expected_ret: value.expected_ret,
        }
    }
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
pub struct SyscallAltBn128Original {
    pub group_op: u64,
    pub input: Vec<u8>,
    pub expected_result: Vec<u8>,
    pub expected_ret: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyscallAltBn128 {
    pub group_op: u64,
    pub input: Vec<u8>,
    pub expected_result: Vec<u8>,
    pub expected_ret: u64,
}
impl From<SyscallAltBn128Original> for SyscallAltBn128 {
    fn from(value: SyscallAltBn128Original) -> Self {
        Self {
            group_op: value.group_op,
            input: value.input,
            expected_result: value.expected_result,
            expected_ret: value.expected_ret,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AltBn128Compression {
    pub group_op: u64,
    pub input: Vec<u8>,
    pub expected_result: Vec<u8>,
    pub expected_ret: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TestCommand {
    ModifyAccount1(ModifyAccount1),
    CreateAccountAndModifySomeData1(CreateAccountAndModifySomeData1),
    SolBigModExp(SolBigModExp),
    SolSecp256k1Recover(SolSecp256k1Recover),
    SolSecp256k1RecoverOriginal(SolSecp256k1RecoverOriginal),
    Keccak256(Keccak256),
    Sha256Original(Sha256Original),
    Sha256(Sha256),
    Blake3(Blake3),
    Poseidon(Poseidon),
    SetGetReturnData(SetGetReturnData),
    CurvePointValidationOriginal(CurvePointValidationOriginal),
    CurvePointValidation(CurvePointValidation),
    CurveGroupOpOriginal(CurveGroupOpOriginal),
    CurveGroupOp(CurveGroupOp),
    CurveMultiscalarMultiplication(CurveMultiscalarMultiplication),
    SyscallAltBn128Original(SyscallAltBn128Original),
    SyscallAltBn128(SyscallAltBn128),
    AltBn128Compression(AltBn128Compression),
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

impl_from!(ModifyAccount1);
impl_from!(CreateAccountAndModifySomeData1);
impl_from!(SolBigModExp);
impl_from!(SolSecp256k1RecoverOriginal);
impl_from!(SolSecp256k1Recover);
impl_from!(Keccak256);
impl_from!(Sha256Original);
impl_from!(Sha256);
impl_from!(Blake3);
impl_from!(Poseidon);
impl_from!(SetGetReturnData);
impl_from!(CurvePointValidationOriginal);
impl_from!(CurvePointValidation);
impl_from!(CurveGroupOpOriginal);
impl_from!(CurveGroupOp);
impl_from!(CurveMultiscalarMultiplication);
impl_from!(SyscallAltBn128Original);
impl_from!(SyscallAltBn128);
impl_from!(AltBn128Compression);
