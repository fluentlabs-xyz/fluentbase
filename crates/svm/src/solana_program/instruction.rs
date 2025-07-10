use alloc::vec::Vec;
use serde::{Deserialize, Serialize};
use solana_bincode::serialize;
pub use solana_instruction::{
    error::InstructionError,
    AccountMeta,
    Instruction,
    ProcessedSiblingInstruction,
    TRANSACTION_LEVEL_STACK_HEIGHT,
};
use solana_sanitize::Sanitize;
use solana_short_vec as short_vec;

/// A compact encoding of an instruction.
///
/// A `CompiledInstruction` is a component of a multi-instruction [`Message`],
/// which is the core of a Solana transaction. It is created during the
/// construction of `Message`. Most users will not interact with it directly.
///
/// [`Message`]: crate::message::Message
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CompiledInstruction {
    /// Index into the transaction keys array indicating the program account that executes this instruction.
    pub program_id_index: u8,
    /// Ordered indices into the transaction keys array indicating which accounts to pass to the program.
    #[serde(with = "short_vec")]
    pub accounts: Vec<u8>,
    /// The program input data.
    #[serde(with = "short_vec")]
    pub data: Vec<u8>,
}

impl Sanitize for CompiledInstruction {}

impl CompiledInstruction {
    pub fn new<T: Serialize>(program_ids_index: u8, data: &T, accounts: Vec<u8>) -> Self {
        let buf = serialize(data).unwrap();
        Self {
            program_id_index: program_ids_index,
            accounts,
            data: buf,
        }
    }
}

/// A non-owning version of [`CompiledInstruction`] that references
/// slices of account indexes and data.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct SVMInstruction<'a> {
    /// Index into the transaction keys array indicating the program account that executes this instruction.
    pub program_id_index: u8,
    /// Ordered indices into the transaction keys array indicating which accounts to pass to the program.
    pub accounts: &'a [u8],
    /// The program input data.
    pub data: &'a [u8],
}

impl<'a> From<&'a CompiledInstruction> for SVMInstruction<'a> {
    fn from(ix: &'a CompiledInstruction) -> Self {
        Self {
            program_id_index: ix.program_id_index,
            accounts: ix.accounts.as_slice(),
            data: ix.data.as_slice(),
        }
    }
}
