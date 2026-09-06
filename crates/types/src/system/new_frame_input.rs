use crate::{
    bincode::{decode_from_bytes, decode_vec, BytesReader, DecodeBytes, ZeroCopyBytes},
    system::JournalLog,
    ExitCode,
};
use alloc::{collections::BTreeMap, vec::Vec};
use alloy_primitives::{Address, Bytes, U256};
use bincode::{
    config::{Config, IntEncoding},
    de::{read::Reader, Decoder},
    error::DecodeError,
};

#[derive(Default, Clone, Debug, PartialEq)]
pub struct RuntimeNewFrameInputV1 {
    pub metadata: Bytes,
    pub input: Bytes,
    pub context: Bytes,
    pub storage: Option<BTreeMap<U256, U256>>,
    // pub balance: Option<U256>,
}

impl bincode::Encode for RuntimeNewFrameInputV1 {
    fn encode<E: bincode::enc::Encoder>(
        &self,
        e: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        bincode::Encode::encode(self.metadata.as_ref(), e)?;
        bincode::Encode::encode(self.input.as_ref(), e)?;
        bincode::Encode::encode(self.context.as_ref(), e)?;
        if let Some(storage) = self.storage.as_ref() {
            bincode::Encode::encode(&(storage.len() as u32), e)?;
            for (k, v) in storage.iter() {
                bincode::Encode::encode(&k.to_le_bytes::<{ U256::BYTES }>(), e)?;
                bincode::Encode::encode(&v.to_le_bytes::<{ U256::BYTES }>(), e)?;
            }
        } else {
            bincode::Encode::encode(&0u32, e)?;
        }
        // if let Some(balance) = self.balance {
        //     bincode::Encode::encode(&balance.to_le_bytes::<{ U256::BYTES }>(), e)?;
        // }
        Ok(())
    }
}

impl<Context> DecodeBytes<Context> for RuntimeNewFrameInputV1 {
    fn decode_bytes<D: Decoder<Context = Context, R = BytesReader>>(
        d: &mut D,
    ) -> Result<Self, bincode::error::DecodeError> {
        let metadata: ZeroCopyBytes = DecodeBytes::<Context>::decode_bytes(d)?;
        let input: ZeroCopyBytes = DecodeBytes::<Context>::decode_bytes(d)?;
        let context: ZeroCopyBytes = DecodeBytes::<Context>::decode_bytes(d)?;
        let storage_len: u32 = bincode::Decode::decode(d)?;
        d.reader()
            .ensure_collection_body(storage_len as usize, 64)?;
        let storage = if storage_len > 0 {
            let mut storage = BTreeMap::<U256, U256>::new();
            for _ in 0..storage_len {
                let k: [u8; 32] = bincode::Decode::decode(d)?;
                let v: [u8; 32] = bincode::Decode::decode(d)?;
                storage.insert(
                    U256::from_le_bytes::<{ U256::BYTES }>(k),
                    U256::from_le_bytes::<{ U256::BYTES }>(v),
                );
            }
            Some(storage)
        } else {
            None
        };

        // let balance: Option<U256> = if d.reader().peek_read(1).is_some() {
        //     let value: [u8; 32] = bincode::Decode::decode(d)?;
        //     Some(U256::from_le_bytes::<{ U256::BYTES }>(value))
        // } else {
        //     None
        // };

        Ok(Self {
            metadata: metadata.into(),
            input: input.into(),
            context: context.into(),
            storage,
            // balance,
        })
    }
}

#[derive(Default, Clone, Debug, PartialEq)]
pub struct RuntimeExecutionOutcomeV1 {
    pub exit_code: ExitCode,
    pub output: Bytes,
    pub storage: Option<BTreeMap<U256, U256>>,
    pub logs: Vec<JournalLog>,
    pub new_metadata: Option<Bytes>,
    pub touched_storage_slots: Option<Vec<U256>>,
    pub transfers: Option<Vec<(Address, U256)>>,
}

impl RuntimeExecutionOutcomeV1 {
    pub fn encode(&self) -> Vec<u8> {
        bincode::encode_to_vec(self, bincode::config::legacy()).unwrap()
    }

    pub fn decode(bytes: Bytes) -> Option<Self> {
        let (result, _bytes_read) = decode_from_bytes(bytes, bincode::config::legacy()).ok()?;
        Some(result)
    }
}

impl bincode::Encode for RuntimeExecutionOutcomeV1 {
    fn encode<E: bincode::enc::Encoder>(
        &self,
        e: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        bincode::Encode::encode(&self.exit_code.into_i32(), e)?;
        bincode::Encode::encode(self.output.as_ref(), e)?;
        if let Some(storage) = self.storage.as_ref() {
            bincode::Encode::encode(&(storage.len() as u32), e)?;
            for (k, v) in storage.iter() {
                bincode::Encode::encode(&k.to_le_bytes::<{ U256::BYTES }>(), e)?;
                bincode::Encode::encode(&v.to_le_bytes::<{ U256::BYTES }>(), e)?;
            }
        } else {
            bincode::Encode::encode(&0u32, e)?;
        }
        bincode::Encode::encode(&self.logs, e)?;
        bincode::Encode::encode(&self.new_metadata.as_ref().map(|v| v.as_ref()), e)?;
        if let Some(touched_storage_slots) = self.touched_storage_slots.as_ref() {
            bincode::Encode::encode(&(touched_storage_slots.len() as u32), e)?;
            for slot in touched_storage_slots.iter() {
                bincode::Encode::encode(&slot.to_le_bytes::<{ U256::BYTES }>(), e)?;
            }
        } else {
            bincode::Encode::encode(&0u32, e)?;
        }
        if let Some(transfers) = self.transfers.as_ref() {
            bincode::Encode::encode(&(transfers.len() as u32), e)?;
            for (recipient, amount) in transfers.iter() {
                let recipient_bytes: [u8; 20] = recipient.as_slice().try_into().unwrap();
                bincode::Encode::encode(&recipient_bytes, e)?;
                bincode::Encode::encode(&amount.to_le_bytes::<{ U256::BYTES }>(), e)?;
            }
        } else {
            bincode::Encode::encode(&0u32, e)?;
        }
        Ok(())
    }
}

impl<Context> DecodeBytes<Context> for RuntimeExecutionOutcomeV1 {
    fn decode_bytes<D: Decoder<Context = Context, R = BytesReader>>(
        d: &mut D,
    ) -> Result<Self, bincode::error::DecodeError> {
        let exit_code: i32 = bincode::Decode::decode(d)?;
        let output: ZeroCopyBytes = DecodeBytes::decode_bytes(d)?;
        let storage_len: u32 = bincode::Decode::decode(d)?;
        d.reader()
            .ensure_collection_body(storage_len as usize, 64)?;
        let storage = if storage_len > 0 {
            let mut storage = BTreeMap::<U256, U256>::new();
            for _ in 0..storage_len {
                let k: [u8; 32] = bincode::Decode::decode(d)?;
                let v: [u8; 32] = bincode::Decode::decode(d)?;
                storage.insert(U256::from_le_bytes(k), U256::from_le_bytes(v));
            }
            Some(storage)
        } else {
            None
        };
        let logs_len: u64 = bincode::Decode::decode(d)?;
        let logs_len =
            usize::try_from(logs_len).map_err(|_| DecodeError::OutsideUsizeRange(logs_len))?;
        // Even an empty log encodes a u32 topic count and a u64 data length.
        let min_log_bytes = match d.config().int_encoding() {
            IntEncoding::Fixed => 12,
            _ => 2,
        };
        let logs = decode_vec(d, logs_len, min_log_bytes, JournalLog::decode_bytes)?;
        let new_metadata: Option<ZeroCopyBytes> = DecodeBytes::decode_bytes(d)?;

        // Backward compatibility: older outcomes do not have this trailing field.
        let touched_storage_slots = if d.reader().peek_read(1).is_some() {
            let touched_slots_len: u32 = bincode::Decode::decode(d)?;
            if touched_slots_len > 0 {
                let touched_storage_slots = decode_vec(d, touched_slots_len as usize, 32, |d| {
                    let slot: [u8; 32] = bincode::Decode::decode(d)?;
                    Ok(U256::from_le_bytes(slot))
                })?;
                Some(touched_storage_slots)
            } else {
                None
            }
        } else {
            None
        };

        // Backward compatibility: older outcomes do not have this trailing field.
        let transfers = if d.reader().peek_read(1).is_some() {
            let transfers_len: u32 = bincode::Decode::decode(d)?;
            if transfers_len > 0 {
                let transfers = decode_vec(d, transfers_len as usize, 52, |d| {
                    let recipient: [u8; 20] = bincode::Decode::decode(d)?;
                    let amount: [u8; 32] = bincode::Decode::decode(d)?;
                    Ok((Address::from(recipient), U256::from_le_bytes(amount)))
                })?;
                Some(transfers)
            } else {
                None
            }
        } else {
            None
        };

        Ok(Self {
            exit_code: ExitCode::from(exit_code),
            output: output.into(),
            storage,
            logs,
            new_metadata: new_metadata.map(Into::into),
            touched_storage_slots,
            transfers,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        bincode::{decode_from_bytes, DecodeBytes},
        system::{
            new_frame_input::{RuntimeExecutionOutcomeV1, RuntimeNewFrameInputV1},
            JournalLog,
        },
        Bytes, ExitCode,
    };
    use alloy_primitives::{bytes, Address, B256, U256};
    use bincode::config::{Configuration, Fixint, LittleEndian};
    use std::collections::BTreeMap;

    pub static BINCODE_CONFIG_DEFAULT: Configuration<LittleEndian, Fixint> =
        bincode::config::legacy();

    pub fn encode<T: bincode::Encode>(entity: &T) -> Result<Vec<u8>, bincode::error::EncodeError> {
        bincode::encode_to_vec(entity, BINCODE_CONFIG_DEFAULT)
    }

    pub fn decode<T: DecodeBytes<()>>(
        src: Bytes,
    ) -> Result<(T, usize), bincode::error::DecodeError> {
        decode_from_bytes(src, BINCODE_CONFIG_DEFAULT)
    }

    #[test]
    fn runtime_outcome_rejects_overflowing_log_count() {
        let mut encoded = RuntimeExecutionOutcomeV1::default().encode();
        // exit code (4), output length (8), storage count (4), then log count (8).
        encoded[16..24].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(decode::<RuntimeExecutionOutcomeV1>(encoded.into()).is_err());
    }

    #[test]
    fn runtime_outcome_rejects_lengths_without_bodies() {
        let empty = RuntimeExecutionOutcomeV1::default().encode();
        // All these prefixes fit in a few bytes. None may allocate based on the count.
        for len in [1_000_000, u32::MAX as u64, u64::MAX] {
            for (name, offset) in [("output", 4), ("logs", 16)] {
                let mut encoded = empty[..offset].to_vec();
                encoded.extend_from_slice(&len.to_le_bytes());
                assert!(
                    decode::<RuntimeExecutionOutcomeV1>(encoded.into()).is_err(),
                    "{name}: {len}"
                );
            }
        }
        for (name, offset) in [("storage", 12), ("touched slots", 25), ("transfers", 29)] {
            let mut encoded = empty[..offset].to_vec();
            encoded.extend_from_slice(&u32::MAX.to_le_bytes());
            assert!(
                decode::<RuntimeExecutionOutcomeV1>(encoded.into()).is_err(),
                "{name}"
            );
        }

        let mut encoded = empty[..24].to_vec();
        encoded.push(1); // Some(new_metadata)
        encoded.extend_from_slice(&u64::MAX.to_le_bytes());
        assert!(decode::<RuntimeExecutionOutcomeV1>(encoded.into()).is_err());

        for (topics, data_len) in [(u32::MAX, 0u64), (0, u64::MAX)] {
            let mut encoded = empty[..16].to_vec();
            encoded.extend_from_slice(&1u64.to_le_bytes()); // One log.
            encoded.extend_from_slice(&topics.to_le_bytes());
            encoded.extend_from_slice(&data_len.to_le_bytes());
            assert!(decode::<RuntimeExecutionOutcomeV1>(encoded.into()).is_err());
        }
    }

    #[test]
    fn runtime_new_frame_rejects_storage_without_body() {
        let mut encoded = encode(&RuntimeNewFrameInputV1::default()).unwrap();
        encoded[24..28].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode::<RuntimeNewFrameInputV1>(encoded.into()).is_err());
    }

    #[test]
    fn runtime_outcome_preserves_legacy_trailing_fields() {
        let empty = RuntimeExecutionOutcomeV1::default();
        let encoded = empty.encode();
        // Before touched slots, before transfers, and the current format.
        for len in [25, 29, 33] {
            let (decoded, consumed) =
                decode::<RuntimeExecutionOutcomeV1>(encoded[..len].to_vec().into()).unwrap();
            assert_eq!(decoded, empty);
            assert_eq!(consumed, len);
        }
        for len in [26, 27, 28, 30, 31, 32] {
            assert!(decode::<RuntimeExecutionOutcomeV1>(encoded[..len].to_vec().into()).is_err());
        }
    }

    #[test]
    fn runtime_outcome_checks_truncated_collection_elements() {
        let outcome = RuntimeExecutionOutcomeV1 {
            output: bytes!("1122"),
            storage: Some([(U256::from(1), U256::from(2))].into()),
            logs: vec![JournalLog {
                topics: vec![B256::repeat_byte(3)],
                data: bytes!("445566"),
            }],
            new_metadata: Some(bytes!("7788")),
            touched_storage_slots: Some(vec![U256::from(1)]),
            transfers: Some(vec![(Address::repeat_byte(4), U256::from(5))]),
            ..Default::default()
        };
        let encoded = outcome.encode();
        // Only complete legacy envelopes ending just before an optional field are valid.
        let before_transfers = encoded.len() - 4 - 52;
        let before_touched = before_transfers - 4 - 32;
        for len in 0..encoded.len() {
            let result = decode::<RuntimeExecutionOutcomeV1>(encoded[..len].to_vec().into());
            assert_eq!(
                result.is_ok(),
                len == before_touched || len == before_transfers,
                "truncated at {len}"
            );
        }
        assert_eq!(
            RuntimeExecutionOutcomeV1::decode(encoded.into()),
            Some(outcome)
        );
    }

    #[test]
    fn runtime_outcome_log_data_remains_zero_copy() {
        let outcome = RuntimeExecutionOutcomeV1 {
            logs: vec![JournalLog {
                topics: vec![B256::repeat_byte(3)],
                data: bytes!("445566"),
            }],
            ..Default::default()
        };
        let encoded: Bytes = outcome.encode().into();
        let (decoded, consumed) = decode::<RuntimeExecutionOutcomeV1>(encoded.clone()).unwrap();
        assert_eq!(decoded, outcome);
        assert_eq!(consumed, encoded.len());
        let data_start = 4 + 8 + 4 + 8 + 4 + 32 + 8;
        assert_eq!(
            decoded.logs[0].data.as_ptr(),
            encoded[data_start..].as_ptr()
        );
    }

    #[test]
    fn runtime_outcome_supports_variable_integer_encoding_and_limits() {
        let outcome = RuntimeExecutionOutcomeV1 {
            logs: vec![JournalLog::default(); 10],
            touched_storage_slots: Some(vec![U256::from(1)]),
            transfers: Some(vec![(Address::ZERO, U256::from(2))]),
            ..Default::default()
        };
        let config = bincode::config::standard();
        let encoded: Bytes = bincode::encode_to_vec(&outcome, config).unwrap().into();
        let (decoded, consumed) =
            decode_from_bytes::<RuntimeExecutionOutcomeV1, _>(encoded.clone(), config).unwrap();
        assert_eq!(decoded, outcome);
        assert_eq!(consumed, encoded.len());
        assert!(decode_from_bytes::<RuntimeExecutionOutcomeV1, _>(
            encoded.clone(),
            config.with_limit::<64>()
        )
        .is_err());
        let (decoded, consumed) = decode_from_bytes::<RuntimeExecutionOutcomeV1, _>(
            encoded.clone(),
            config.with_limit::<1024>(),
        )
        .unwrap();
        assert_eq!(decoded, outcome);
        assert_eq!(consumed, encoded.len());
    }

    #[test]
    fn test_runtime_new_frame_input_v1_encode_decode() {
        let mut storage = BTreeMap::new();
        let mut v = RuntimeNewFrameInputV1 {
            metadata: [1, 2, 3].into(),
            input: [4, 5, 6, 7].into(),
            context: [8, 9, 10, 11, 12].into(),
            storage: Some(storage.clone()),
            // balance: Some(U256::from(13u64)),
        };
        let v_encoded: Bytes = encode(&v).unwrap().into();
        let (v_decoded, bytes_count): (RuntimeNewFrameInputV1, usize) =
            decode(v_encoded.clone()).unwrap();
        assert_eq!(v_encoded.len(), bytes_count);
        v.storage = None;
        assert_eq!(v_decoded, v);

        storage.insert(U256::from_le_bytes([1; 32]), U256::from_le_bytes([2; 32]));
        storage.insert(U256::from_le_bytes([3; 32]), U256::from_le_bytes([4; 32]));
        let v = RuntimeNewFrameInputV1 {
            metadata: [1, 2, 3].into(),
            input: [4, 5, 6, 7].into(),
            context: [8, 9, 10, 11, 12].into(),
            storage: Some(storage.clone()),
            // balance: Some(U256::from(42u64)),
        };
        let v_encoded: Bytes = encode(&v).unwrap().into();
        let (v_decoded, read_count) = decode::<RuntimeNewFrameInputV1>(v_encoded.clone()).unwrap();
        assert_eq!(v_encoded.len(), read_count);
        assert_eq!(v_decoded, v);
    }

    #[test]
    fn test_runtime_new_frame_input_v1_zero_copy_decode() {
        let v = RuntimeNewFrameInputV1 {
            metadata: [1, 2, 3, 4, 5].into(),
            ..Default::default()
        };
        let v_encoded: Bytes = encode(&v).unwrap().into();
        let (v_decoded, bytes_count): (RuntimeNewFrameInputV1, usize) =
            decode(v_encoded.clone()).unwrap();
        assert_eq!(v_encoded.len(), bytes_count);
        assert_eq!(v_decoded, v);
        // Make sure `metadata` is in the same range as v_encoded.
        assert!(
            v_decoded.metadata.as_ptr() as usize >= v_encoded.as_ptr() as usize
                && (v_decoded.metadata.as_ptr() as usize)
                    < v_encoded.as_ptr() as usize + v_encoded.len()
        );
    }

    #[test]
    fn test_encode_decode_none_metadata() {
        let v = RuntimeExecutionOutcomeV1 {
            exit_code: ExitCode::BadConversionToInteger,
            output: bytes!("112233"),
            storage: None,
            logs: vec![],
            new_metadata: None,
            touched_storage_slots: None,
            transfers: None,
        };
        let v_encoded = v.encode();
        let v_decoded = RuntimeExecutionOutcomeV1::decode(v_encoded.into()).unwrap();
        assert_eq!(v_decoded, v);
    }

    #[test]
    fn test_runtime_output_v1_encode_decode() {
        let mut storage = BTreeMap::new();
        let mut logs = Vec::new();
        let mut v = RuntimeExecutionOutcomeV1 {
            exit_code: ExitCode::PrecompileError,
            output: [1, 2, 3].into(),
            storage: Some(storage.clone()),
            logs: logs.clone(),
            new_metadata: Some(bytes!("112233")),
            touched_storage_slots: Some(vec![U256::from(1u64)]),
            transfers: Some(vec![(Address::repeat_byte(0x22), U256::from(234))]),
        };
        let v_encoded: Bytes = encode(&v).unwrap().into();
        let (v_decoded, read_count) =
            decode::<RuntimeExecutionOutcomeV1>(v_encoded.clone()).unwrap();
        assert_eq!(v_encoded.len(), read_count);
        v.storage = None;
        assert_eq!(v_decoded, v);

        storage.insert(U256::from_le_bytes([1; 32]), U256::from_le_bytes([2; 32]));
        storage.insert(U256::from_le_bytes([3; 32]), U256::from_le_bytes([4; 32]));
        logs.push(JournalLog {
            topics: vec![B256::repeat_byte(4), B256::repeat_byte(7)],
            data: vec![].into(),
        });
        logs.push(JournalLog {
            topics: vec![],
            data: vec![4, 5].into(),
        });
        logs.push(JournalLog {
            topics: vec![B256::repeat_byte(87), B256::repeat_byte(23)],
            data: vec![4, 5].into(),
        });
        let v = RuntimeExecutionOutcomeV1 {
            exit_code: ExitCode::CallDepthOverflow,
            output: [1, 2, 3].into(),
            storage: Some(storage.clone()),
            logs,
            new_metadata: None,
            touched_storage_slots: None,
            transfers: None,
        };
        let v_encoded: Bytes = encode(&v).unwrap().into();
        let (v_decoded, read_count) =
            decode::<RuntimeExecutionOutcomeV1>(v_encoded.clone()).unwrap();
        assert_eq!(v_encoded.len(), read_count);
        assert_eq!(v_decoded, v);
    }
}
