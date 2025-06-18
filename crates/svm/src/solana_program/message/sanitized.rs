use crate::{
    solana_program::{
        ed25519_program,
        instruction::CompiledInstruction,
        message::{
            legacy,
            versions::{v0, v0::LoadedAddresses},
            AccountKeys,
            MessageHeader,
        },
        nonce::NONCED_TX_MARKER_IX_INDEX,
        secp256k1_program,
        sysvar::instructions::{BorrowedAccountMeta, BorrowedInstruction},
    },
    system_instruction::SystemInstruction,
    system_program,
};
use alloc::{borrow::Cow, vec::Vec};
use core::convert::TryFrom;
use hashbrown::HashSet;
use solana_bincode::limited_deserialize;
use solana_hash::Hash;
use solana_instruction::error::InstructionError;
use solana_pubkey::Pubkey;
use solana_sanitize::Sanitize;
#[deprecated(
    since = "2.1.0",
    note = "Use solana_transaction_error::SanitizeMessageError instead"
)]
pub use solana_transaction_error::SanitizeMessageError;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LegacyMessage<'a> {
    /// Legacy message
    pub message: Cow<'a, legacy::Message>,
    /// List of boolean with same length as account_keys(), each boolean value indicates if
    /// corresponding account key is writable or not.
    pub is_writable_account_cache: Vec<bool>,
}

impl<'a> LegacyMessage<'a> {
    pub fn new(message: legacy::Message, reserved_account_keys: &HashSet<Pubkey>) -> Self {
        let is_writable_account_cache = message
            .account_keys
            .iter()
            .enumerate()
            .map(|(i, _key)| {
                message.is_writable_index(i)
                    && !reserved_account_keys.contains(&message.account_keys[i])
                    && !message.demote_program_id(i)
            })
            .collect::<Vec<_>>();
        Self {
            message: Cow::Owned(message),
            is_writable_account_cache,
        }
    }

    pub fn has_duplicates(&self) -> bool {
        self.message.has_duplicates()
    }

    pub fn is_key_called_as_program(&self, key_index: usize) -> bool {
        self.message.is_key_called_as_program(key_index)
    }

    /// Inspect all message keys for the bpf upgradeable loader
    pub fn is_upgradeable_loader_present(&self) -> bool {
        self.message.is_upgradeable_loader_present()
    }

    /// Returns the full list of account keys.
    pub fn account_keys(&self) -> AccountKeys {
        AccountKeys::new(&self.message.account_keys, None)
    }

    pub fn is_writable(&self, index: usize) -> bool {
        *self.is_writable_account_cache.get(index).unwrap_or(&false)
    }
}

/// Sanitized message of a transaction.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum SanitizedMessage {
    /// Sanitized legacy message
    Legacy(LegacyMessage<'static>),
    // /// Sanitized version #0 message with dynamically loaded addresses
    V0(v0::LoadedMessage<'static>),
}

impl SanitizedMessage {
    // /// Create a sanitized message from a sanitized versioned message.
    // /// If the input message uses address tables, attempt to look up the
    // /// address for each table index.
    // pub fn try_new(
    //     sanitized_msg: SanitizedVersionedMessage,
    //     address_loader: impl AddressLoader,
    //     reserved_account_keys: &HashSet<Pubkey>,
    // ) -> Result<Self, SanitizeMessageError> {
    //     Ok(match sanitized_msg.message {
    //         VersionedMessage::Legacy(message) => {
    //             SanitizedMessage::Legacy(LegacyMessage::new(message, reserved_account_keys))
    //         }
    //         VersionedMessage::V0(message) => {
    //             let loaded_addresses =
    //                 address_loader.load_addresses(&message.address_table_lookups)?;
    //             SanitizedMessage::V0(v0::LoadedMessage::new(
    //                 message,
    //                 loaded_addresses,
    //                 reserved_account_keys,
    //             ))
    //         }
    //     })
    // }

    /// Create a sanitized legacy message
    pub fn try_from_legacy_message(
        message: legacy::Message,
        reserved_account_keys: &HashSet<Pubkey>,
    ) -> Result<Self, SanitizeMessageError> {
        message.sanitize()?;
        Ok(Self::Legacy(LegacyMessage::new(
            message,
            reserved_account_keys,
        )))
    }

    /// Return true if this message contains duplicate account keys
    pub fn has_duplicates(&self) -> bool {
        match self {
            SanitizedMessage::Legacy(message) => message.has_duplicates(),
            SanitizedMessage::V0(message) => message.has_duplicates(),
        }
    }

    /// Message header which identifies the number of signer and writable or
    /// readonly accounts
    pub fn header(&self) -> &MessageHeader {
        match self {
            Self::Legacy(legacy_message) => &legacy_message.message.header,
            Self::V0(loaded_msg) => &loaded_msg.message.header,
        }
    }

    /// Returns a legacy message if this sanitized message wraps one
    pub fn legacy_message(&self) -> Option<&legacy::Message> {
        if let Self::Legacy(legacy_message) = &self {
            Some(&legacy_message.message)
        } else {
            None
        }
    }

    /// Returns the fee payer for the transaction
    pub fn fee_payer(&self) -> &Pubkey {
        self.account_keys()
            .get(0)
            .expect("sanitized messages always have a fee payer at index 0")
    }

    /// The hash of a recent block, used for timing out a transaction
    pub fn recent_blockhash(&self) -> &Hash {
        match self {
            Self::Legacy(legacy_message) => &legacy_message.message.recent_blockhash,
            Self::V0(loaded_msg) => &loaded_msg.message.recent_blockhash,
        }
    }

    /// Program instructions that will be executed in sequence and committed in
    /// one atomic transaction if all succeed.
    pub fn instructions(&self) -> &[CompiledInstruction] {
        match self {
            Self::Legacy(legacy_message) => &legacy_message.message.instructions,
            Self::V0(loaded_msg) => &loaded_msg.message.instructions,
        }
    }

    /// Program instructions iterator which includes each instruction's program
    /// id.
    pub fn program_instructions_iter(
        &self,
    ) -> impl Iterator<Item = (&Pubkey, &CompiledInstruction)> + Clone {
        self.instructions().iter().map(move |ix| {
            (
                self.account_keys()
                    .get(usize::from(ix.program_id_index))
                    .expect("program id index is sanitized"),
                ix,
            )
        })
    }

    /// Returns the list of account keys that are loaded for this message.
    pub fn account_keys(&self) -> AccountKeys {
        match self {
            Self::Legacy(message) => message.account_keys(),
            Self::V0(message) => message.account_keys(),
        }
    }

    /// Returns the list of account keys used for account lookup tables.
    pub fn message_address_table_lookups(&self) -> &[v0::MessageAddressTableLookup] {
        match self {
            Self::Legacy(_message) => &[],
            Self::V0(message) => &message.message.address_table_lookups,
        }
    }

    /// Returns true if the account at the specified index is an input to some
    /// program instruction in this message.
    #[deprecated(since = "2.0.0", note = "Please use `is_instruction_account` instead")]
    pub fn is_key_passed_to_program(&self, key_index: usize) -> bool {
        self.is_instruction_account(key_index)
    }

    /// Returns true if the account at the specified index is an input to some
    /// program instruction in this message.
    pub fn is_instruction_account(&self, key_index: usize) -> bool {
        if let Ok(key_index) = u8::try_from(key_index) {
            self.instructions()
                .iter()
                .any(|ix| ix.accounts.contains(&key_index))
        } else {
            false
        }
    }

    /// Returns true if the account at the specified index is invoked as a
    /// program in this message.
    pub fn is_invoked(&self, key_index: usize) -> bool {
        match self {
            Self::Legacy(message) => message.is_key_called_as_program(key_index),
            Self::V0(message) => message.is_key_called_as_program(key_index),
        }
    }

    /// Returns true if the account at the specified index is not invoked as a
    /// program or, if invoked, is passed to a program.
    #[deprecated(
        since = "2.0.0",
        note = "Please use `is_invoked` and `is_instruction_account` instead"
    )]
    pub fn is_non_loader_key(&self, key_index: usize) -> bool {
        !self.is_invoked(key_index) || self.is_instruction_account(key_index)
    }

    /// Returns true if the account at the specified index is writable by the
    /// instructions in this message.
    pub fn is_writable(&self, index: usize) -> bool {
        match self {
            Self::Legacy(message) => message.is_writable(index),
            Self::V0(message) => message.is_writable(index),
        }
    }

    /// Returns true if the account at the specified index signed this
    /// message.
    pub fn is_signer(&self, index: usize) -> bool {
        index < usize::from(self.header().num_required_signatures)
    }

    /// Return the resolved addresses for this message if it has any.
    fn loaded_lookup_table_addresses(&self) -> Option<&LoadedAddresses> {
        match &self {
            SanitizedMessage::V0(message) => Some(&message.loaded_addresses),
            _ => None,
        }
    }

    /// Return the number of readonly accounts loaded by this message.
    pub fn num_readonly_accounts(&self) -> usize {
        let loaded_readonly_addresses = self
            .loaded_lookup_table_addresses()
            .map(|keys| keys.readonly.len())
            .unwrap_or_default();
        loaded_readonly_addresses
            .saturating_add(usize::from(self.header().num_readonly_signed_accounts))
            .saturating_add(usize::from(self.header().num_readonly_unsigned_accounts))
    }

    /// Decompile message instructions without cloning account keys
    pub fn decompile_instructions(&self) -> Vec<BorrowedInstruction> {
        let account_keys = self.account_keys();
        self.program_instructions_iter()
            .map(|(program_id, instruction)| {
                let accounts = instruction
                    .accounts
                    .iter()
                    .map(|account_index| {
                        let account_index = *account_index as usize;
                        BorrowedAccountMeta {
                            is_signer: self.is_signer(account_index),
                            is_writable: self.is_writable(account_index),
                            pubkey: account_keys.get(account_index).unwrap(),
                        }
                    })
                    .collect();

                BorrowedInstruction {
                    accounts,
                    data: &instruction.data,
                    program_id,
                }
            })
            .collect()
    }

    /// Inspect all message keys for the bpf upgradeable loader
    pub fn is_upgradeable_loader_present(&self) -> bool {
        match self {
            Self::Legacy(message) => message.is_upgradeable_loader_present(),
            Self::V0(message) => message.is_upgradeable_loader_present(),
        }
    }

    /// Get a list of signers for the instruction at the given index
    pub fn get_ix_signers(&self, ix_index: usize) -> impl Iterator<Item = &Pubkey> {
        self.instructions()
            .get(ix_index)
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

    /// If the message uses a durable nonce, return the pubkey of the nonce account
    pub fn get_durable_nonce(&self) -> Option<&Pubkey> {
        self.instructions()
            .get(NONCED_TX_MARKER_IX_INDEX as usize)
            .filter(
                |ix| match self.account_keys().get(ix.program_id_index as usize) {
                    Some(program_id) => system_program::check_id(program_id),
                    _ => false,
                },
            )
            .filter(|ix| {
                matches!(
                    limited_deserialize::<4, _>(
                        &ix.data /* serialized size of AdvanceNonceAccount */
                    )
                    .map_err(|_| InstructionError::InvalidInstructionData),
                    Ok(SystemInstruction::AdvanceNonceAccount)
                )
            })
            .and_then(|ix| {
                ix.accounts.first().and_then(|idx| {
                    let idx = *idx as usize;
                    if !self.is_writable(idx) {
                        None
                    } else {
                        self.account_keys().get(idx)
                    }
                })
            })
    }

    #[deprecated(
        since = "2.1.0",
        note = "Please use `SanitizedMessage::num_total_signatures` instead."
    )]
    pub fn num_signatures(&self) -> u64 {
        self.num_total_signatures()
    }

    /// Returns the total number of signatures in the message.
    /// This includes required transaction signatures as well as any
    /// pre-compile signatures that are attached in instructions.
    pub fn num_total_signatures(&self) -> u64 {
        self.get_signature_details().total_signatures()
    }

    /// Returns the number of requested write-locks in this message.
    /// This does not consider if write-locks are demoted.
    pub fn num_write_locks(&self) -> u64 {
        self.account_keys()
            .len()
            .saturating_sub(self.num_readonly_accounts()) as u64
    }

    /// return detailed signature counts
    pub fn get_signature_details(&self) -> TransactionSignatureDetails {
        let mut transaction_signature_details = TransactionSignatureDetails {
            num_transaction_signatures: u64::from(self.header().num_required_signatures),
            ..TransactionSignatureDetails::default()
        };

        // counting the number of pre-processor operations separately
        for (program_id, instruction) in self.program_instructions_iter() {
            if secp256k1_program::check_id(program_id) {
                if let Some(num_verifies) = instruction.data.first() {
                    transaction_signature_details.num_secp256k1_instruction_signatures =
                        transaction_signature_details
                            .num_secp256k1_instruction_signatures
                            .saturating_add(u64::from(*num_verifies));
                }
            } else if ed25519_program::check_id(program_id) {
                if let Some(num_verifies) = instruction.data.first() {
                    transaction_signature_details.num_ed25519_instruction_signatures =
                        transaction_signature_details
                            .num_ed25519_instruction_signatures
                            .saturating_add(u64::from(*num_verifies));
                }
            }
        }

        transaction_signature_details
    }
}

/// Transaction signature details including the number of transaction signatures
/// and precompile signatures.
#[derive(Debug, Default)]
pub struct TransactionSignatureDetails {
    num_transaction_signatures: u64,
    num_secp256k1_instruction_signatures: u64,
    num_ed25519_instruction_signatures: u64,
}

impl TransactionSignatureDetails {
    pub fn new(
        num_transaction_signatures: u64,
        num_secp256k1_instruction_signatures: u64,
        num_ed25519_instruction_signatures: u64,
    ) -> Self {
        Self {
            num_transaction_signatures,
            num_secp256k1_instruction_signatures,
            num_ed25519_instruction_signatures,
        }
    }

    /// return total number of signature, treating pre-processor operations as signature
    pub fn total_signatures(&self) -> u64 {
        self.num_transaction_signatures
            .saturating_add(self.num_secp256k1_instruction_signatures)
            .saturating_add(self.num_ed25519_instruction_signatures)
    }

    /// return the number of transaction signatures
    pub fn num_transaction_signatures(&self) -> u64 {
        self.num_transaction_signatures
    }

    /// return the number of secp256k1 instruction signatures
    pub fn num_secp256k1_instruction_signatures(&self) -> u64 {
        self.num_secp256k1_instruction_signatures
    }

    /// return the number of ed25519 instruction signatures
    pub fn num_ed25519_instruction_signatures(&self) -> u64 {
        self.num_ed25519_instruction_signatures
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solana_program::{ed25519_program, secp256k1_program};
    // use crate::message::v0;

    #[test]
    fn test_try_from_legacy_message() {
        let legacy_message_with_no_signers = legacy::Message {
            account_keys: vec![Pubkey::new_unique()],
            ..legacy::Message::default()
        };

        assert_eq!(
            SanitizedMessage::try_from_legacy_message(
                legacy_message_with_no_signers,
                &HashSet::default(),
            )
            .err(),
            Some(SanitizeMessageError::IndexOutOfBounds),
        );
    }

    #[test]
    fn test_is_non_loader_key() {
        #![allow(deprecated)]
        let key0 = Pubkey::new_unique();
        let key1 = Pubkey::new_unique();
        let loader_key = Pubkey::new_unique();
        let instructions = vec![
            CompiledInstruction::new(1, &(), vec![0]),
            CompiledInstruction::new(2, &(), vec![0, 1]),
        ];

        let message = SanitizedMessage::try_from_legacy_message(
            legacy::Message::new_with_compiled_instructions(
                1,
                0,
                2,
                vec![key0, key1, loader_key],
                Hash::default(),
                instructions,
            ),
            &HashSet::default(),
        )
        .unwrap();

        assert!(message.is_non_loader_key(0));
        assert!(message.is_non_loader_key(1));
        assert!(!message.is_non_loader_key(2));
    }

    #[test]
    fn test_num_readonly_accounts() {
        let key0 = Pubkey::new_unique();
        let key1 = Pubkey::new_unique();
        let key2 = Pubkey::new_unique();
        let key3 = Pubkey::new_unique();
        let key4 = Pubkey::new_unique();
        let key5 = Pubkey::new_unique();

        let legacy_message = SanitizedMessage::try_from_legacy_message(
            legacy::Message {
                header: MessageHeader {
                    num_required_signatures: 2,
                    num_readonly_signed_accounts: 1,
                    num_readonly_unsigned_accounts: 1,
                },
                account_keys: vec![key0, key1, key2, key3],
                ..legacy::Message::default()
            },
            &HashSet::default(),
        )
        .unwrap();

        assert_eq!(legacy_message.num_readonly_accounts(), 2);

        let v0_message = SanitizedMessage::V0(v0::LoadedMessage::new(
            v0::Message {
                header: MessageHeader {
                    num_required_signatures: 2,
                    num_readonly_signed_accounts: 1,
                    num_readonly_unsigned_accounts: 1,
                },
                account_keys: vec![key0, key1, key2, key3],
                ..v0::Message::default()
            },
            LoadedAddresses {
                writable: vec![key4],
                readonly: vec![key5],
            },
            &HashSet::default(),
        ));

        assert_eq!(v0_message.num_readonly_accounts(), 3);
    }

    #[test]
    fn test_get_ix_signers() {
        let signer0 = Pubkey::new_unique();
        let signer1 = Pubkey::new_unique();
        let non_signer = Pubkey::new_unique();
        let loader_key = Pubkey::new_unique();
        let instructions = vec![
            CompiledInstruction::new(3, &(), vec![2, 0]),
            CompiledInstruction::new(3, &(), vec![0, 1]),
            CompiledInstruction::new(3, &(), vec![0, 0]),
        ];

        let message = SanitizedMessage::try_from_legacy_message(
            legacy::Message::new_with_compiled_instructions(
                2,
                1,
                2,
                vec![signer0, signer1, non_signer, loader_key],
                Hash::default(),
                instructions,
            ),
            &HashSet::default(),
        )
        .unwrap();

        assert_eq!(
            message.get_ix_signers(0).collect::<HashSet<_>>(),
            HashSet::from_iter([&signer0])
        );
        assert_eq!(
            message.get_ix_signers(1).collect::<HashSet<_>>(),
            HashSet::from_iter([&signer0, &signer1])
        );
        assert_eq!(
            message.get_ix_signers(2).collect::<HashSet<_>>(),
            HashSet::from_iter([&signer0])
        );
        assert_eq!(
            message.get_ix_signers(3).collect::<HashSet<_>>(),
            HashSet::default()
        );
    }

    #[test]
    #[allow(clippy::get_first)]
    fn test_is_writable_account_cache() {
        let key0 = Pubkey::new_unique();
        let key1 = Pubkey::new_unique();
        let key2 = Pubkey::new_unique();
        let key3 = Pubkey::new_unique();
        let key4 = Pubkey::new_unique();
        let key5 = Pubkey::new_unique();

        let legacy_message = SanitizedMessage::try_from_legacy_message(
            legacy::Message {
                header: MessageHeader {
                    num_required_signatures: 2,
                    num_readonly_signed_accounts: 1,
                    num_readonly_unsigned_accounts: 1,
                },
                account_keys: vec![key0, key1, key2, key3],
                ..legacy::Message::default()
            },
            &HashSet::default(),
        )
        .unwrap();
        match legacy_message {
            SanitizedMessage::Legacy(message) => {
                assert_eq!(
                    message.is_writable_account_cache.len(),
                    message.account_keys().len()
                );
                assert!(message.is_writable_account_cache.get(0).unwrap());
                assert!(!message.is_writable_account_cache.get(1).unwrap());
                assert!(message.is_writable_account_cache.get(2).unwrap());
                assert!(!message.is_writable_account_cache.get(3).unwrap());
            }
            _ => {
                panic!("Expect to be SanitizedMessage::LegacyMessage")
            }
        }

        let v0_message = SanitizedMessage::V0(v0::LoadedMessage::new(
            v0::Message {
                header: MessageHeader {
                    num_required_signatures: 2,
                    num_readonly_signed_accounts: 1,
                    num_readonly_unsigned_accounts: 1,
                },
                account_keys: vec![key0, key1, key2, key3],
                ..v0::Message::default()
            },
            LoadedAddresses {
                writable: vec![key4],
                readonly: vec![key5],
            },
            &HashSet::default(),
        ));
        match v0_message {
            SanitizedMessage::V0(message) => {
                assert_eq!(
                    message.is_writable_account_cache.len(),
                    message.account_keys().len()
                );
                assert!(message.is_writable_account_cache.get(0).unwrap());
                assert!(!message.is_writable_account_cache.get(1).unwrap());
                assert!(message.is_writable_account_cache.get(2).unwrap());
                assert!(!message.is_writable_account_cache.get(3).unwrap());
                assert!(message.is_writable_account_cache.get(4).unwrap());
                assert!(!message.is_writable_account_cache.get(5).unwrap());
            }
            _ => {
                panic!("Expect to be SanitizedMessage::V0")
            }
        }
    }

    #[test]
    fn test_get_signature_details() {
        let key0 = Pubkey::new_unique();
        let key1 = Pubkey::new_unique();
        let loader_key = Pubkey::new_unique();

        let loader_instr = CompiledInstruction::new(2, &(), vec![0, 1]);
        let mock_secp256k1_instr = CompiledInstruction::new(3, &[1u8; 10], vec![]);
        let mock_ed25519_instr = CompiledInstruction::new(4, &[5u8; 10], vec![]);

        let message = SanitizedMessage::try_from_legacy_message(
            legacy::Message::new_with_compiled_instructions(
                2,
                1,
                2,
                vec![
                    key0,
                    key1,
                    loader_key,
                    secp256k1_program::id(),
                    ed25519_program::id(),
                ],
                Hash::default(),
                vec![
                    loader_instr,
                    mock_secp256k1_instr.clone(),
                    mock_ed25519_instr,
                    mock_secp256k1_instr,
                ],
            ),
            &HashSet::new(),
        )
        .unwrap();

        let signature_details = message.get_signature_details();
        // expect 2 required transaction signatures
        assert_eq!(2, signature_details.num_transaction_signatures);
        // expect 2 secp256k1 instruction signatures - 1 for each mock_secp2561k1_instr
        assert_eq!(2, signature_details.num_secp256k1_instruction_signatures);
        // expect 5 ed25519 instruction signatures from mock_ed25519_instr
        assert_eq!(5, signature_details.num_ed25519_instruction_signatures);
    }
}
