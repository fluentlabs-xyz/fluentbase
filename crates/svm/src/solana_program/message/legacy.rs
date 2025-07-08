//! The original and current Solana message format.
//!
//! This crate defines two versions of `Message` in their own modules:
//! [`legacy`] and [`v0`]. `legacy` is the current version as of Solana 1.10.0.
//! `v0` is a [future message format] that encodes more account keys into a
//! transaction than the legacy format.
//!
//! [`legacy`]: crate::message::legacy
//! [`v0`]: crate::message::v0
//! [future message format]: https://docs.solanalabs.com/proposals/versioned-transactions

#![allow(clippy::arithmetic_side_effects)]

use crate::{
    bpf_loader_deprecated,
    solana_program::{
        instruction::CompiledInstruction,
        message::{compiled_keys::CompiledKeys, MessageHeader},
        sysvar,
    },
    system_instruction,
    system_program,
};
use alloc::vec::Vec;
#[allow(deprecated)]
pub use builtins::{BUILTIN_PROGRAMS_KEYS, MAYBE_BUILTIN_KEY_OR_SYSVAR};
use core::{convert::TryFrom, str::FromStr};
use hashbrown::HashSet;
use serde::{Deserialize, Serialize};
use solana_bincode::serialize;
use solana_hash::Hash;
use solana_instruction::Instruction;
use solana_pubkey::Pubkey;
use solana_sanitize::{Sanitize, SanitizeError};
use solana_short_vec as short_vec;

#[deprecated(
    since = "2.0.0",
    note = "please use `solana_sdk::reserved_account_keys::ReservedAccountKeys` instead"
)]
#[allow(deprecated)]
mod builtins {
    use super::*;
    use crate::bpf_loader;
    use lazy_static::lazy_static;
    use solana_pubkey::Pubkey;

    lazy_static! {
        pub static ref BUILTIN_PROGRAMS_KEYS: [Pubkey; 9] = {
            let parse = |s| Pubkey::from_str(s).unwrap();
            [
                parse("Config1111111111111111111111111111111111111"),
                parse("Feature111111111111111111111111111111111111"),
                parse("NativeLoader1111111111111111111111111111111"),
                parse("Stake11111111111111111111111111111111111111"),
                parse("StakeConfig11111111111111111111111111111111"),
                parse("Vote111111111111111111111111111111111111111"),
                system_program::id(),
                bpf_loader::id(),
                bpf_loader_deprecated::id(),
                // bpf_loader_upgradeable::id(),
            ]
        };
    }

    lazy_static! {
        // Each element of a key is a u8. We use key[0] as an index into this table of 256 boolean
        // elements, to store whether or not the first element of any key is present in the static
        // lists of built-in-program keys or system ids. By using this lookup table, we can very
        // quickly determine that a key under consideration cannot be in either of these lists (if
        // the value is "false"), or might be in one of these lists (if the value is "true")
        pub static ref MAYBE_BUILTIN_KEY_OR_SYSVAR: [bool; 256] = {
            let mut temp_table: [bool; 256] = [false; 256];
            BUILTIN_PROGRAMS_KEYS.iter().for_each(|key| temp_table[key.as_ref()[0] as usize] = true);
            sysvar::ALL_IDS.iter().for_each(|key| temp_table[key.as_ref()[0] as usize] = true);
            temp_table
        };
    }
}

#[deprecated(
    since = "2.0.0",
    note = "please use `solana_sdk::reserved_account_keys::ReservedAccountKeys::is_reserved` instead"
)]
#[allow(deprecated)]
pub fn is_builtin_key_or_sysvar(key: &Pubkey) -> bool {
    if MAYBE_BUILTIN_KEY_OR_SYSVAR[key.as_ref()[0] as usize] {
        return sysvar::is_sysvar_id(key) || BUILTIN_PROGRAMS_KEYS.contains(key);
    }
    false
}

fn position(keys: &[Pubkey], key: &Pubkey) -> u8 {
    keys.iter().position(|k| k == key).unwrap() as u8
}

fn compile_instruction(ix: &Instruction, keys: &[Pubkey]) -> CompiledInstruction {
    let accounts: Vec<_> = ix
        .accounts
        .iter()
        .map(|account_meta| position(keys, &account_meta.pubkey))
        .collect();

    CompiledInstruction {
        program_id_index: position(keys, &ix.program_id),
        data: ix.data.clone(),
        accounts,
    }
}

fn compile_instructions(ixs: &[Instruction], keys: &[Pubkey]) -> Vec<CompiledInstruction> {
    ixs.iter().map(|ix| compile_instruction(ix, keys)).collect()
}

/// A Solana transaction message (legacy).
///
/// See the [`message`] module documentation for further description.
///
/// [`message`]: crate::message
///
/// Some constructors accept an optional `payer`, the account responsible for
/// paying the cost of executing a transaction. In most cases, callers should
/// specify the payer explicitly in these constructors. In some cases though,
/// the caller is not _required_ to specify the payer, but is still allowed to:
/// in the `Message` structure, the first account is always the fee-payer, so if
/// the caller has knowledge that the first account of the constructed
/// transaction's `Message` is both a signer and the expected fee-payer, then
/// redundantly specifying the fee-payer is not strictly required.
// NOTE: Serialization-related changes must be paired with the custom serialization
// for versioned messages in the `RemainingLegacyMessage` struct.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Serialize, Deserialize, Default, Debug, PartialEq, Eq, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    /// The message header, identifying signed and read-only `account_keys`.
    // NOTE: Serialization-related changes must be paired with the direct read at sigverify.
    pub header: MessageHeader,

    /// All the account keys used by this transaction.
    #[serde(with = "short_vec")]
    pub account_keys: Vec<Pubkey>,

    /// The id of a recent ledger entry.
    pub recent_blockhash: Hash,

    /// Programs that will be executed in sequence and committed in one atomic transaction if all
    /// succeed.
    #[serde(with = "short_vec")]
    pub instructions: Vec<CompiledInstruction>,
}

/// wasm-bindgen version of the Message struct.
/// This duplication is required until https://github.com/rustwasm/wasm-bindgen/issues/3671
/// is fixed. This must not diverge from the regular non-wasm Message struct.
#[cfg(target_arch = "wasm32")]
#[derive(Serialize, Deserialize, Default, Debug, PartialEq, Eq, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub header: MessageHeader,

    #[serde(with = "short_vec")]
    pub account_keys: Vec<Pubkey>,

    /// The id of a recent ledger entry.
    pub recent_blockhash: Hash,

    #[serde(with = "short_vec")]
    pub instructions: Vec<CompiledInstruction>,
}

impl Sanitize for Message {
    fn sanitize(&self) -> Result<(), SanitizeError> {
        // signing area and read-only non-signing area should not overlap
        if self.header.num_required_signatures as usize
            + self.header.num_readonly_unsigned_accounts as usize
            > self.account_keys.len()
        {
            return Err(SanitizeError::IndexOutOfBounds);
        }

        // there should be at least 1 RW fee-payer account.
        if self.header.num_readonly_signed_accounts >= self.header.num_required_signatures {
            return Err(SanitizeError::IndexOutOfBounds);
        }

        for ci in &self.instructions {
            if ci.program_id_index as usize >= self.account_keys.len() {
                return Err(SanitizeError::IndexOutOfBounds);
            }
            // A program cannot be a payer.
            if ci.program_id_index == 0 {
                return Err(SanitizeError::IndexOutOfBounds);
            }
            for ai in &ci.accounts {
                if *ai as usize >= self.account_keys.len() {
                    return Err(SanitizeError::IndexOutOfBounds);
                }
            }
        }
        self.account_keys.sanitize()?;
        self.recent_blockhash.sanitize()?;
        self.instructions.sanitize()?;
        Ok(())
    }
}

impl Message {
    pub fn new(instructions: &[Instruction], payer: Option<&Pubkey>) -> Self {
        Self::new_with_blockhash(instructions, payer, &Hash::default())
    }
    pub fn new_with_blockhash(
        instructions: &[Instruction],
        payer: Option<&Pubkey>,
        blockhash: &Hash,
    ) -> Self {
        let compiled_keys = CompiledKeys::compile(instructions, payer.cloned());
        let (header, account_keys) = compiled_keys
            .try_into_message_components()
            .expect("overflow when compiling message keys");
        let instructions = compile_instructions(instructions, &account_keys);
        Self::new_with_compiled_instructions(
            header.num_required_signatures,
            header.num_readonly_signed_accounts,
            header.num_readonly_unsigned_accounts,
            account_keys,
            *blockhash,
            instructions,
        )
    }

    pub fn new_with_nonce(
        mut instructions: Vec<Instruction>,
        payer: Option<&Pubkey>,
        nonce_account_pubkey: &Pubkey,
        nonce_authority_pubkey: &Pubkey,
    ) -> Self {
        let nonce_ix =
            system_instruction::advance_nonce_account(nonce_account_pubkey, nonce_authority_pubkey);
        instructions.insert(0, nonce_ix);
        Self::new(&instructions, payer)
    }

    pub fn new_with_compiled_instructions(
        num_required_signatures: u8,
        num_readonly_signed_accounts: u8,
        num_readonly_unsigned_accounts: u8,
        account_keys: Vec<Pubkey>,
        recent_blockhash: Hash,
        instructions: Vec<CompiledInstruction>,
    ) -> Self {
        Self {
            header: MessageHeader {
                num_required_signatures,
                num_readonly_signed_accounts,
                num_readonly_unsigned_accounts,
            },
            account_keys,
            recent_blockhash,
            instructions,
        }
    }

    /// Compute the blake3 hash of this transaction's message.
    #[cfg(test)]
    pub fn hash(&self) -> Hash {
        let message_bytes = self.serialize();
        Self::hash_raw_message(&message_bytes)
    }

    /// Compute the blake3 hash of a raw transaction message.
    #[cfg(test)]
    pub fn hash_raw_message(message_bytes: &[u8]) -> Hash {
        use solana_hash::HASH_BYTES;
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"solana-tx-message-v1");
        hasher.update(message_bytes);
        let hash_bytes: [u8; HASH_BYTES] = hasher.finalize().into();
        hash_bytes.into()
    }

    pub fn compile_instruction(&self, ix: &Instruction) -> CompiledInstruction {
        compile_instruction(ix, &self.account_keys)
    }

    pub fn serialize(&self) -> Vec<u8> {
        serialize(self).unwrap()
    }

    pub fn program_id(&self, instruction_index: usize) -> Option<&Pubkey> {
        Some(
            &self.account_keys[self.instructions.get(instruction_index)?.program_id_index as usize],
        )
    }

    pub fn program_index(&self, instruction_index: usize) -> Option<usize> {
        Some(self.instructions.get(instruction_index)?.program_id_index as usize)
    }

    pub fn program_ids(&self) -> Vec<&Pubkey> {
        self.instructions
            .iter()
            .map(|ix| &self.account_keys[ix.program_id_index as usize])
            .collect()
    }

    /// Returns true if the account at the specified index is an account input
    /// to some program instruction in this message.
    pub fn is_instruction_account(&self, key_index: usize) -> bool {
        if let Ok(key_index) = u8::try_from(key_index) {
            self.instructions
                .iter()
                .any(|ix| ix.accounts.contains(&key_index))
        } else {
            false
        }
    }

    pub fn is_key_called_as_program(&self, key_index: usize) -> bool {
        if let Ok(key_index) = u8::try_from(key_index) {
            self.instructions
                .iter()
                .any(|ix| ix.program_id_index == key_index)
        } else {
            false
        }
    }

    #[deprecated(
        since = "2.0.0",
        note = "Please use `is_key_called_as_program` and `is_instruction_account` directly"
    )]
    #[cfg(test)]
    pub fn is_non_loader_key(&self, key_index: usize) -> bool {
        !self.is_key_called_as_program(key_index) || self.is_instruction_account(key_index)
    }

    pub fn program_position(&self, index: usize) -> Option<usize> {
        let program_ids = self.program_ids();
        program_ids
            .iter()
            .position(|&&pubkey| pubkey == self.account_keys[index])
    }

    pub fn maybe_executable(&self, i: usize) -> bool {
        self.program_position(i).is_some()
    }

    pub fn demote_program_id(&self, i: usize) -> bool {
        self.is_key_called_as_program(i)
    }

    /// Returns true if the account at the specified index was requested to be
    /// writable. This method should not be used directly.
    pub(super) fn is_writable_index(&self, i: usize) -> bool {
        i < (self.header.num_required_signatures - self.header.num_readonly_signed_accounts)
            as usize
            || (i >= self.header.num_required_signatures as usize
                && i < self.account_keys.len()
                    - self.header.num_readonly_unsigned_accounts as usize)
    }

    /// Returns true if the account at the specified index is writable by the
    /// instructions in this message. Since the dynamic set of reserved accounts
    /// isn't used here to demote write locks, this shouldn't be used in the
    /// runtime.
    #[deprecated(since = "2.0.0", note = "Please use `is_maybe_writable` instead")]
    #[allow(deprecated)]
    #[cfg(test)]
    pub fn is_writable(&self, i: usize) -> bool {
        (self.is_writable_index(i))
            && !is_builtin_key_or_sysvar(&self.account_keys[i])
            && !self.demote_program_id(i)
    }

    /// Returns true if the account at the specified index is writable by the
    /// instructions in this message. The `reserved_account_keys` param has been
    /// optional to allow clients to approximate writability without requiring
    /// fetching the latest set of reserved account keys. If this method is
    /// called by the runtime, the latest set of reserved account keys must be
    /// passed.
    pub fn is_maybe_writable(
        &self,
        i: usize,
        reserved_account_keys: Option<&HashSet<Pubkey>>,
    ) -> bool {
        (self.is_writable_index(i))
            && !self.is_account_maybe_reserved(i, reserved_account_keys)
            && !self.demote_program_id(i)
    }

    /// Returns true if the account at the specified index is in the optional
    /// reserved account keys set.
    fn is_account_maybe_reserved(
        &self,
        key_index: usize,
        reserved_account_keys: Option<&HashSet<Pubkey>>,
    ) -> bool {
        let mut is_maybe_reserved = false;
        if let Some(reserved_account_keys) = reserved_account_keys {
            if let Some(key) = self.account_keys.get(key_index) {
                is_maybe_reserved = reserved_account_keys.contains(key);
            }
        }
        is_maybe_reserved
    }

    pub fn is_signer(&self, i: usize) -> bool {
        i < self.header.num_required_signatures as usize
    }

    pub fn signer_keys(&self) -> Vec<&Pubkey> {
        // Clamp in case we're working on un-`sanitize()`ed input
        let last_key = self
            .account_keys
            .len()
            .min(self.header.num_required_signatures as usize);
        self.account_keys[..last_key].iter().collect()
    }

    /// Returns `true` if `account_keys` has any duplicate keys.
    pub fn has_duplicates(&self) -> bool {
        // Note: This is an O(n^2) algorithm, but requires no heap allocations. The benchmark
        // `bench_has_duplicates` in benches/message_processor.rs shows that this implementation is
        // ~50 times faster than using HashSet for very short slices.
        for i in 1..self.account_keys.len() {
            #[allow(clippy::arithmetic_side_effects)]
            if self.account_keys[i..].contains(&self.account_keys[i - 1]) {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    #![allow(deprecated)]

    use super::*;
    use crate::{
        hash,
        solana_program::{instruction::AccountMeta, message::MESSAGE_HEADER_LENGTH},
    };
    use solana_bincode::serialized_size;

    #[test]
    fn test_builtin_program_keys() {
        let keys: HashSet<Pubkey> = BUILTIN_PROGRAMS_KEYS.iter().copied().collect();
        assert_eq!(keys.len(), 9);
        for k in keys {
            let k = format!("{k}");
            assert!(k.ends_with("11111111111111111111111"));
        }
    }

    #[test]
    fn test_builtin_program_keys_abi_freeze() {
        // Once the feature is flipped on, we can't further modify
        // BUILTIN_PROGRAMS_KEYS without the risk of breaking consensus.
        let builtins = format!("{:?}", *BUILTIN_PROGRAMS_KEYS);
        assert_eq!(
            format!("{}", hash::hash(builtins.as_bytes())),
            "3NR3Qmx9ynhQvZqvcFHutZ79Yjb6zyaGwLWvC8zU6kEB"
        );
    }

    #[test]
    // Ensure there's a way to calculate the number of required signatures.
    fn test_message_signed_keys_len() {
        let program_id = Pubkey::default();
        let id0 = Pubkey::default();
        let ix = Instruction::new_with_bincode(program_id, &0, vec![AccountMeta::new(id0, false)]);
        let message = Message::new(&[ix], None);
        assert_eq!(message.header.num_required_signatures, 0);

        let ix = Instruction::new_with_bincode(program_id, &0, vec![AccountMeta::new(id0, true)]);
        let message = Message::new(&[ix], Some(&id0));
        assert_eq!(message.header.num_required_signatures, 1);
    }

    #[test]
    fn test_message_kitchen_sink() {
        let program_id0 = Pubkey::new_unique();
        let program_id1 = Pubkey::new_unique();
        let id0 = Pubkey::default();
        let id1 = Pubkey::new_unique();
        let message = Message::new(
            &[
                Instruction::new_with_bincode(program_id0, &0, vec![AccountMeta::new(id0, false)]),
                Instruction::new_with_bincode(program_id1, &0, vec![AccountMeta::new(id1, true)]),
                Instruction::new_with_bincode(program_id0, &0, vec![AccountMeta::new(id1, false)]),
            ],
            Some(&id1),
        );
        assert_eq!(
            message.instructions[0],
            CompiledInstruction::new(2, &0, vec![1])
        );
        assert_eq!(
            message.instructions[1],
            CompiledInstruction::new(3, &0, vec![0])
        );
        assert_eq!(
            message.instructions[2],
            CompiledInstruction::new(2, &0, vec![0])
        );
    }

    #[test]
    fn test_message_payer_first() {
        let program_id = Pubkey::default();
        let payer = Pubkey::new_unique();
        let id0 = Pubkey::default();

        let ix = Instruction::new_with_bincode(program_id, &0, vec![AccountMeta::new(id0, false)]);
        let message = Message::new(&[ix], Some(&payer));
        assert_eq!(message.header.num_required_signatures, 1);

        let ix = Instruction::new_with_bincode(program_id, &0, vec![AccountMeta::new(id0, true)]);
        let message = Message::new(&[ix], Some(&payer));
        assert_eq!(message.header.num_required_signatures, 2);

        let ix = Instruction::new_with_bincode(
            program_id,
            &0,
            vec![AccountMeta::new(payer, true), AccountMeta::new(id0, true)],
        );
        let message = Message::new(&[ix], Some(&payer));
        assert_eq!(message.header.num_required_signatures, 2);
    }

    #[test]
    fn test_program_position() {
        let program_id0 = Pubkey::default();
        let program_id1 = Pubkey::new_unique();
        let id = Pubkey::new_unique();
        let message = Message::new(
            &[
                Instruction::new_with_bincode(program_id0, &0, vec![AccountMeta::new(id, false)]),
                Instruction::new_with_bincode(program_id1, &0, vec![AccountMeta::new(id, true)]),
            ],
            Some(&id),
        );
        assert_eq!(message.program_position(0), None);
        assert_eq!(message.program_position(1), Some(0));
        assert_eq!(message.program_position(2), Some(1));
    }

    #[test]
    fn test_is_writable() {
        let key0 = Pubkey::new_unique();
        let key1 = Pubkey::new_unique();
        let key2 = Pubkey::new_unique();
        let key3 = Pubkey::new_unique();
        let key4 = Pubkey::new_unique();
        let key5 = Pubkey::new_unique();

        let message = Message {
            header: MessageHeader {
                num_required_signatures: 3,
                num_readonly_signed_accounts: 2,
                num_readonly_unsigned_accounts: 1,
            },
            account_keys: vec![key0, key1, key2, key3, key4, key5],
            recent_blockhash: Hash::default(),
            instructions: vec![],
        };
        assert!(message.is_writable(0));
        assert!(!message.is_writable(1));
        assert!(!message.is_writable(2));
        assert!(message.is_writable(3));
        assert!(message.is_writable(4));
        assert!(!message.is_writable(5));
    }

    #[test]
    fn test_is_maybe_writable() {
        let key0 = Pubkey::new_unique();
        let key1 = Pubkey::new_unique();
        let key2 = Pubkey::new_unique();
        let key3 = Pubkey::new_unique();
        let key4 = Pubkey::new_unique();
        let key5 = Pubkey::new_unique();

        let message = Message {
            header: MessageHeader {
                num_required_signatures: 3,
                num_readonly_signed_accounts: 2,
                num_readonly_unsigned_accounts: 1,
            },
            account_keys: vec![key0, key1, key2, key3, key4, key5],
            recent_blockhash: Hash::default(),
            instructions: vec![],
        };

        let reserved_account_keys = HashSet::from([key3]);

        assert!(message.is_maybe_writable(0, Some(&reserved_account_keys)));
        assert!(!message.is_maybe_writable(1, Some(&reserved_account_keys)));
        assert!(!message.is_maybe_writable(2, Some(&reserved_account_keys)));
        assert!(!message.is_maybe_writable(3, Some(&reserved_account_keys)));
        assert!(message.is_maybe_writable(3, None));
        assert!(message.is_maybe_writable(4, Some(&reserved_account_keys)));
        assert!(!message.is_maybe_writable(5, Some(&reserved_account_keys)));
        assert!(!message.is_maybe_writable(6, Some(&reserved_account_keys)));
    }

    #[test]
    fn test_is_account_maybe_reserved() {
        let key0 = Pubkey::new_unique();
        let key1 = Pubkey::new_unique();

        let message = Message {
            account_keys: vec![key0, key1],
            ..Message::default()
        };

        let reserved_account_keys = HashSet::from([key1]);

        assert!(!message.is_account_maybe_reserved(0, Some(&reserved_account_keys)));
        assert!(message.is_account_maybe_reserved(1, Some(&reserved_account_keys)));
        assert!(!message.is_account_maybe_reserved(2, Some(&reserved_account_keys)));
        assert!(!message.is_account_maybe_reserved(0, None));
        assert!(!message.is_account_maybe_reserved(1, None));
        assert!(!message.is_account_maybe_reserved(2, None));
    }

    #[test]
    fn test_program_ids() {
        let key0 = Pubkey::new_unique();
        let key1 = Pubkey::new_unique();
        let loader2 = Pubkey::new_unique();
        let instructions = vec![CompiledInstruction::new(2, &(), vec![0, 1])];
        let message = Message::new_with_compiled_instructions(
            1,
            0,
            2,
            vec![key0, key1, loader2],
            Hash::default(),
            instructions,
        );
        assert_eq!(message.program_ids(), vec![&loader2]);
    }

    #[test]
    fn test_is_instruction_account() {
        let key0 = Pubkey::new_unique();
        let key1 = Pubkey::new_unique();
        let loader2 = Pubkey::new_unique();
        let instructions = vec![CompiledInstruction::new(2, &(), vec![0, 1])];
        let message = Message::new_with_compiled_instructions(
            1,
            0,
            2,
            vec![key0, key1, loader2],
            Hash::default(),
            instructions,
        );

        assert!(message.is_instruction_account(0));
        assert!(message.is_instruction_account(1));
        assert!(!message.is_instruction_account(2));
    }

    #[test]
    fn test_is_non_loader_key() {
        #![allow(deprecated)]
        let key0 = Pubkey::new_unique();
        let key1 = Pubkey::new_unique();
        let loader2 = Pubkey::new_unique();
        let instructions = vec![CompiledInstruction::new(2, &(), vec![0, 1])];
        let message = Message::new_with_compiled_instructions(
            1,
            0,
            2,
            vec![key0, key1, loader2],
            Hash::default(),
            instructions,
        );
        assert!(message.is_non_loader_key(0));
        assert!(message.is_non_loader_key(1));
        assert!(!message.is_non_loader_key(2));
    }

    #[test]
    fn test_message_header_len_constant() {
        assert_eq!(
            serialized_size(&MessageHeader::default()).unwrap(),
            MESSAGE_HEADER_LENGTH
        );
    }

    #[test]
    fn test_message_hash() {
        // when this test fails, it's most likely due to a new serialized format of a message.
        // in this case, the domain prefix `solana-tx-message-v1` should be updated.
        let program_id0 = Pubkey::from_str("4uQeVj5tqViQh7yWWGStvkEG1Zmhx6uasJtWCJziofM").unwrap();
        let program_id1 = Pubkey::from_str("8opHzTAnfzRpPEx21XtnrVTX28YQuCpAjcn1PczScKh").unwrap();
        let id0 = Pubkey::from_str("CiDwVBFgWV9E5MvXWoLgnEgn2hK7rJikbvfWavzAQz3").unwrap();
        let id1 = Pubkey::from_str("GcdayuLaLyrdmUu324nahyv33G5poQdLUEZ1nEytDeP").unwrap();
        let id2 = Pubkey::from_str("LX3EUdRUBUa3TbsYXLEUdj9J3prXkWXvLYSWyYyc2Jj").unwrap();
        let id3 = Pubkey::from_str("QRSsyMWN1yHT9ir42bgNZUNZ4PdEhcSWCrL2AryKpy5").unwrap();
        let instructions = vec![
            Instruction::new_with_bincode(program_id0, &0, vec![AccountMeta::new(id0, false)]),
            Instruction::new_with_bincode(program_id0, &0, vec![AccountMeta::new(id1, true)]),
            Instruction::new_with_bincode(
                program_id1,
                &0,
                vec![AccountMeta::new_readonly(id2, false)],
            ),
            Instruction::new_with_bincode(
                program_id1,
                &0,
                vec![AccountMeta::new_readonly(id3, true)],
            ),
        ];

        let message = Message::new(&instructions, Some(&id1));
        assert_eq!(
            message.hash(),
            Hash::from_str("7VWCF4quo2CcWQFNUayZiorxpiR5ix8YzLebrXKf3fMF").unwrap()
        )
    }
}
