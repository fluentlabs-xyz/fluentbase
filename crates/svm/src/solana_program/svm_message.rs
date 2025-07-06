pub mod sanitized_message;

use crate::{
    solana_program::{
        instruction::SVMInstruction,
        message::AccountKeys,
        nonce::NONCED_TX_MARKER_IX_INDEX,
    },
    system_program,
};
use solana_hash::Hash;
use solana_pubkey::Pubkey;
use {
    // crate::{
    //     instruction::SVMInstruction, message_address_table_lookup::SVMMessageAddressTableLookup,
    // },
    core::fmt::Debug,
    // solana_sdk::{
    //     hash::Hash, message::AccountKeys, nonce::NONCED_TX_MARKER_IX_INDEX, pubkey::Pubkey,
    //     system_program,
    // },
};
// mod sanitized_message;
// mod sanitized_transaction;

// - Debug to support legacy logging
pub trait SVMMessage: Debug + Clone {
    // /// Returns the total number of signatures in the message.
    // /// This includes required transaction signatures as well as any
    // /// pre-compile signatures that are attached in instructions.
    // fn num_total_signatures(&self) -> u64;

    // /// Returns the number of requested write-locks in this message.
    // /// This does not consider if write-locks are demoted.
    // fn num_write_locks(&self) -> u64;

    /// Return the recent blockhash.
    // fn recent_blockhash(&self) -> &Hash;

    /// Return the number of instructions in the message.
    fn num_instructions(&self) -> usize;

    /// Return an iterator over the instructions in the message.
    fn instructions_iter(&self) -> impl Iterator<Item = SVMInstruction>;

    /// Return an iterator over the instructions in the message, paired with
    /// the pubkey of the program.
    fn program_instructions_iter(&self) -> impl Iterator<Item = (&Pubkey, SVMInstruction)> + Clone;

    /// Return the account keys.
    fn account_keys(&self) -> AccountKeys;

    /// Return the fee-payer
    fn fee_payer(&self) -> &Pubkey;

    /// Returns `true` if the account at `index` is writable.
    fn is_writable(&self, index: usize) -> bool;

    /// Returns `true` if the account at `index` is signer.
    fn is_signer(&self, index: usize) -> bool;

    // /// Returns true if the account at the specified index is invoked as a
    // /// program in top-level instructions of this message.
    // fn is_invoked(&self, key_index: usize) -> bool;

    /// Returns true if the account at the specified index is an input to some
    /// program instruction in this message.
    fn is_instruction_account(&self, key_index: usize) -> bool {
        if let Ok(key_index) = u8::try_from(key_index) {
            self.instructions_iter()
                .any(|ix| ix.accounts.contains(&key_index))
        } else {
            false
        }
    }

    // /// If the message uses a durable nonce, return the pubkey of the nonce account
    // fn get_durable_nonce(&self) -> Option<&Pubkey> {
    //     let account_keys = self.account_keys();
    //     self.instructions_iter()
    //         .nth(usize::from(NONCED_TX_MARKER_IX_INDEX))
    //         .filter(
    //             |ix| match account_keys.get(usize::from(ix.program_id_index)) {
    //                 Some(program_id) => system_program::check_id(program_id),
    //                 _ => false,
    //             },
    //         )
    //         .filter(|ix| {
    //             /// Serialized value of [`SystemInstruction::AdvanceNonceAccount`].
    //             const SERIALIZED_ADVANCE_NONCE_ACCOUNT: [u8; 4] = 4u32.to_le_bytes();
    //             const SERIALIZED_SIZE: usize = SERIALIZED_ADVANCE_NONCE_ACCOUNT.len();
    //
    //             ix.data
    //                 .get(..SERIALIZED_SIZE)
    //                 .map(|data| data == SERIALIZED_ADVANCE_NONCE_ACCOUNT)
    //                 .unwrap_or(false)
    //         })
    //         .and_then(|ix| {
    //             ix.accounts.first().and_then(|idx| {
    //                 let index = usize::from(*idx);
    //                 if !self.is_writable(index) {
    //                     None
    //                 } else {
    //                     account_keys.get(index)
    //                 }
    //             })
    //         })
    // }

    /// For the instruction at `index`, return an iterator over input accounts
    /// that are signers.
    fn get_ix_signers(&self, index: usize) -> impl Iterator<Item = &Pubkey> {
        self.instructions_iter()
            .nth(index)
            .into_iter()
            .flat_map(|ix| {
                ix.accounts
                    .iter()
                    .copied()
                    .map(usize::from)
                    .filter(|index| self.is_signer(*index))
                    .filter_map(|signer_index| self.account_keys().get(signer_index))
            })
    }

    // /// Get the number of lookup tables.
    // fn num_lookup_tables(&self) -> usize;

    // /// Get message address table lookups used in the message
    // fn message_address_table_lookups(&self) -> impl Iterator<Item = SVMMessageAddressTableLookup>;
}
