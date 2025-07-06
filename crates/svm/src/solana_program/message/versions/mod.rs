use crate::solana_program::message::{Message as LegacyMessage, MessageHeader};
use alloc::{fmt, vec::Vec};
use hashbrown::HashSet;
use serde::{
    de::{self, Deserializer, SeqAccess, Unexpected, Visitor},
    ser::{SerializeTuple, Serializer},
    Deserialize,
    Serialize,
};
use solana_bincode::serialize;
use solana_hash::{Hash, HASH_BYTES};
use solana_pubkey::Pubkey;
use solana_sanitize::{Sanitize, SanitizeError};
use solana_short_vec as short_vec;

mod sanitized;
pub mod v0;

use crate::solana_program::{
    instruction::CompiledInstruction,
    message::versions::v0::MessageAddressTableLookup,
};
pub use sanitized::*;

/// Bit mask that indicates whether a serialized message is versioned.
pub const MESSAGE_VERSION_PREFIX: u8 = 0x80;

/// Either a legacy message or a v0 message.
///
/// # Serialization
///
/// If the first bit is set, the remaining 7 bits will be used to determine
/// which message version is serialized starting from version `0`. If the first
/// is bit is not set, all bytes are used to encode the legacy `Message`
/// format.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum VersionedMessage {
    Legacy(LegacyMessage),
    V0(v0::Message),
}

impl VersionedMessage {
    pub fn sanitize(&self) -> Result<(), SanitizeError> {
        match self {
            Self::Legacy(message) => message.sanitize(),
            Self::V0(message) => message.sanitize(),
        }
    }

    pub fn header(&self) -> &MessageHeader {
        match self {
            Self::Legacy(message) => &message.header,
            Self::V0(message) => &message.header,
        }
    }

    pub fn static_account_keys(&self) -> &[Pubkey] {
        match self {
            Self::Legacy(message) => &message.account_keys,
            Self::V0(message) => &message.account_keys,
        }
    }

    pub fn address_table_lookups(&self) -> Option<&[MessageAddressTableLookup]> {
        match self {
            Self::Legacy(_) => None,
            Self::V0(message) => Some(&message.address_table_lookups),
        }
    }

    /// Returns true if the account at the specified index signed this
    /// message.
    pub fn is_signer(&self, index: usize) -> bool {
        index < usize::from(self.header().num_required_signatures)
    }

    /// Returns true if the account at the specified index is writable by the
    /// instructions in this message. Since dynamically loaded addresses can't
    /// have write locks demoted without loading addresses, this shouldn't be
    /// used in the runtime.
    pub fn is_maybe_writable(
        &self,
        index: usize,
        reserved_account_keys: Option<&HashSet<Pubkey>>,
    ) -> bool {
        match self {
            Self::Legacy(message) => message.is_maybe_writable(index, reserved_account_keys),
            Self::V0(message) => message.is_maybe_writable(index, reserved_account_keys),
        }
    }

    #[deprecated(since = "2.0.0", note = "Please use `is_instruction_account` instead")]
    pub fn is_key_passed_to_program(&self, key_index: usize) -> bool {
        self.is_instruction_account(key_index)
    }

    /// Returns true if the account at the specified index is an input to some
    /// program instruction in this message.
    fn is_instruction_account(&self, key_index: usize) -> bool {
        if let Ok(key_index) = u8::try_from(key_index) {
            self.instructions()
                .iter()
                .any(|ix| ix.accounts.contains(&key_index))
        } else {
            false
        }
    }

    pub fn is_invoked(&self, key_index: usize) -> bool {
        match self {
            Self::Legacy(message) => message.is_key_called_as_program(key_index),
            Self::V0(message) => message.is_key_called_as_program(key_index),
        }
    }

    /// Returns true if the account at the specified index is not invoked as a
    /// program or, if invoked, is passed to a program.
    pub fn is_non_loader_key(&self, key_index: usize) -> bool {
        !self.is_invoked(key_index) || self.is_instruction_account(key_index)
    }

    pub fn recent_blockhash(&self) -> &Hash {
        match self {
            Self::Legacy(message) => &message.recent_blockhash,
            Self::V0(message) => &message.recent_blockhash,
        }
    }

    pub fn set_recent_blockhash(&mut self, recent_blockhash: Hash) {
        match self {
            Self::Legacy(message) => message.recent_blockhash = recent_blockhash,
            Self::V0(message) => message.recent_blockhash = recent_blockhash,
        }
    }

    /// Program instructions that will be executed in sequence and committed in
    /// one atomic transaction if all succeed.
    pub fn instructions(&self) -> &[CompiledInstruction] {
        match self {
            Self::Legacy(message) => &message.instructions,
            Self::V0(message) => &message.instructions,
        }
    }

    pub fn serialize(&self) -> Vec<u8> {
        serialize(self).unwrap()
    }

    /// Compute the blake3 hash of this transaction's message
    pub fn hash(&self) -> Hash {
        let message_bytes = self.serialize();
        Self::hash_raw_message(&message_bytes)
    }

    /// Compute the blake3 hash of a raw transaction message
    pub fn hash_raw_message(message_bytes: &[u8]) -> Hash {
        // use blake3::traits::digest::Digest;
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"solana-tx-message-v1");
        hasher.update(message_bytes);
        let hash_bytes: [u8; HASH_BYTES] = hasher.finalize().into();
        hash_bytes.into()
    }
}

impl Default for VersionedMessage {
    fn default() -> Self {
        Self::Legacy(LegacyMessage::default())
    }
}

impl serde::Serialize for VersionedMessage {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Legacy(message) => {
                let mut seq = serializer.serialize_tuple(1)?;
                seq.serialize_element(message)?;
                seq.end()
            }
            Self::V0(message) => {
                let mut seq = serializer.serialize_tuple(2)?;
                seq.serialize_element(&MESSAGE_VERSION_PREFIX)?;
                seq.serialize_element(message)?;
                seq.end()
            }
        }
    }
}

enum MessagePrefix {
    Legacy(u8),
    Versioned(u8),
}

impl<'de> serde::Deserialize<'de> for MessagePrefix {
    fn deserialize<D>(deserializer: D) -> Result<MessagePrefix, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PrefixVisitor;

        impl<'de> Visitor<'de> for PrefixVisitor {
            type Value = MessagePrefix;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("message prefix byte")
            }

            // Serde's integer visitors bubble up to u64 so check the prefix
            // with this function instead of visit_u8. This approach is
            // necessary because serde_json directly calls visit_u64 for
            // unsigned integers.
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<MessagePrefix, E> {
                if value > u8::MAX as u64 {
                    Err(de::Error::invalid_type(Unexpected::Unsigned(value), &self))?;
                }

                let byte = value as u8;
                if byte & MESSAGE_VERSION_PREFIX != 0 {
                    Ok(MessagePrefix::Versioned(byte & !MESSAGE_VERSION_PREFIX))
                } else {
                    Ok(MessagePrefix::Legacy(byte))
                }
            }
        }

        deserializer.deserialize_u8(PrefixVisitor)
    }
}

impl<'de> serde::Deserialize<'de> for VersionedMessage {
    fn deserialize<D>(deserializer: D) -> Result<VersionedMessage, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct MessageVisitor;

        impl<'de> Visitor<'de> for MessageVisitor {
            type Value = VersionedMessage;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("message bytes")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<VersionedMessage, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let prefix: MessagePrefix = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;

                match prefix {
                    MessagePrefix::Legacy(num_required_signatures) => {
                        // The remaining fields of the legacy Message struct after the first byte.
                        #[derive(Serialize, Deserialize)]
                        struct RemainingLegacyMessage {
                            pub num_readonly_signed_accounts: u8,
                            pub num_readonly_unsigned_accounts: u8,
                            #[serde(with = "short_vec")]
                            pub account_keys: Vec<Pubkey>,
                            pub recent_blockhash: Hash,
                            #[serde(with = "short_vec")]
                            pub instructions: Vec<CompiledInstruction>,
                        }

                        let message: RemainingLegacyMessage =
                            seq.next_element()?.ok_or_else(|| {
                                // will never happen since tuple length is always 2
                                de::Error::invalid_length(1, &self)
                            })?;

                        Ok(VersionedMessage::Legacy(LegacyMessage {
                            header: MessageHeader {
                                num_required_signatures,
                                num_readonly_signed_accounts: message.num_readonly_signed_accounts,
                                num_readonly_unsigned_accounts: message
                                    .num_readonly_unsigned_accounts,
                            },
                            account_keys: message.account_keys,
                            recent_blockhash: message.recent_blockhash,
                            instructions: message.instructions,
                        }))
                    }
                    MessagePrefix::Versioned(version) => {
                        match version {
                            // 0 => {
                            //     Ok(VersionedMessage::V0(seq.next_element()?.ok_or_else(
                            //         || {
                            //             // will never happen since tuple length is always 2
                            //             de::Error::invalid_length(1, &self)
                            //         },
                            //     )?))
                            // }
                            127 => {
                                // 0xff is used as the first byte of the off-chain messages
                                // which corresponds to version 127 of the versioned messages.
                                // This explicit check is added to prevent the usage of version 127
                                // in the runtime as a valid transaction.
                                Err(de::Error::custom("off-chain messages are not accepted"))
                            }
                            _ => Err(de::Error::invalid_value(
                                de::Unexpected::Unsigned(version as u64),
                                &"a valid transaction message version",
                            )),
                        }
                    }
                }
            }
        }

        deserializer.deserialize_tuple(2, MessageVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        solana_program::instruction::{AccountMeta, Instruction},
        // message::v0::MessageAddressTableLookup,
    };
    use solana_bincode::deserialize;
    use solana_pubkey::Pubkey;

    #[test]
    fn test_legacy_message_serialization() {
        let program_id0 = Pubkey::new_unique();
        let program_id1 = Pubkey::new_unique();
        let id0 = Pubkey::new_unique();
        let id1 = Pubkey::new_unique();
        let id2 = Pubkey::new_unique();
        let id3 = Pubkey::new_unique();
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

        let mut message = LegacyMessage::new(&instructions, Some(&id1));
        message.recent_blockhash = Hash::new_unique();
        let wrapped_message = VersionedMessage::Legacy(message.clone());

        // bincode
        {
            let bytes = serialize(&message).unwrap();
            assert_eq!(bytes, serialize(&wrapped_message).unwrap());

            let message_from_bytes: LegacyMessage = deserialize(&bytes).unwrap();
            let wrapped_message_from_bytes: VersionedMessage = deserialize(&bytes).unwrap();

            assert_eq!(message, message_from_bytes);
            assert_eq!(wrapped_message, wrapped_message_from_bytes);
        }

        // serde_json
        {
            let string = serde_json::to_string(&message).unwrap();
            let message_from_string: LegacyMessage = serde_json::from_str(&string).unwrap();
            assert_eq!(message, message_from_string);
        }
    }

    // #[test]
    // fn test_versioned_message_serialization() {
    //     let message = VersionedMessage::V0(v0::Message {
    //         header: MessageHeader {
    //             num_required_signatures: 1,
    //             num_readonly_signed_accounts: 0,
    //             num_readonly_unsigned_accounts: 0,
    //         },
    //         recent_blockhash: Hash::new_unique(),
    //         account_keys: vec![Pubkey::new_unique()],
    //         address_table_lookups: vec![
    //             MessageAddressTableLookup {
    //                 account_key: Pubkey::new_unique(),
    //                 writable_indexes: vec![1],
    //                 readonly_indexes: vec![0],
    //             },
    //             MessageAddressTableLookup {
    //                 account_key: Pubkey::new_unique(),
    //                 writable_indexes: vec![0],
    //                 readonly_indexes: vec![1],
    //             },
    //         ],
    //         instructions: vec![CompiledInstruction {
    //             program_id_index: 1,
    //             accounts: vec![0, 2, 3, 4],
    //             data: vec![],
    //         }],
    //     });
    //
    //     let bytes = bincode_serialize(&message).unwrap();
    //     let message_from_bytes: VersionedMessage = bincode_deserialize(&bytes).unwrap();
    //     assert_eq!(message, message_from_bytes);
    //
    //     let string = serde_json::to_string(&message).unwrap();
    //     let message_from_string: VersionedMessage = serde_json::from_str(&string).unwrap();
    //     assert_eq!(message, message_from_string);
    // }
}
