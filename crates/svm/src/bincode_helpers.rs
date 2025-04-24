#[cfg(test)]
pub mod tests {
    use crate::system_instruction::SystemInstruction;
    use solana_bincode::{bincode_serialize, limited_deserialize};

    #[test]
    fn test_limited_deserialize_advance_nonce_account() {
        let item = SystemInstruction::AdvanceNonceAccount;
        let mut serialized = bincode_serialize(&item).unwrap();

        assert_eq!(
            serialized.len(),
            4,
            "`SanitizedMessage::get_durable_nonce()` may need a change"
        );

        assert_eq!(
            limited_deserialize::<4, SystemInstruction>(&serialized)
                .as_ref()
                .unwrap(),
            &item
        );
        assert!(limited_deserialize::<3, SystemInstruction>(&serialized).is_err());

        serialized.push(0);
        assert_eq!(
            limited_deserialize::<4, SystemInstruction>(&serialized)
                .as_ref()
                .unwrap(),
            &item
        );
    }
}
