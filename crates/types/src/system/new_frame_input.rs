use crate::{
    bincode::{decode_from_bytes, BytesReader, DecodeBytes, ZeroCopyBytes},
    system::JournalLog,
    ExitCode,
};
use alloc::{collections::BTreeMap, vec::Vec};
use alloy_primitives::{Address, Bytes, U256};
use bincode::de::{read::Reader, Decoder};

#[derive(Default, Clone, Debug, PartialEq)]
pub struct RuntimeNewFrameInputV1 {
    pub metadata: Bytes,
    pub input: Bytes,
    pub context: Bytes,
    pub storage: Option<BTreeMap<U256, U256>>,
    pub balance: Option<U256>,
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
        if let Some(balance) = self.balance {
            bincode::Encode::encode(balance.as_le_slice(), e)?;
        }
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
        let balance: Option<U256> = match bincode::Decode::decode(d) {
            Ok(bytes) => Some(bytes),
            Err(bincode::error::DecodeError::UnexpectedEnd { .. }) => None,
            Err(err) => return Err(err),
        }
        .map(|value| U256::from_le_bytes::<32>(value));
        Ok(Self {
            metadata: metadata.into(),
            input: input.into(),
            context: context.into(),
            storage,
            balance,
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
        let logs: Vec<JournalLog> = bincode::Decode::decode(d)?;
        let new_metadata: Option<ZeroCopyBytes> = DecodeBytes::decode_bytes(d)?;

        // Backward compatibility: older outcomes do not have this trailing field.
        let touched_storage_slots = if d.reader().peek_read(1).is_some() {
            let touched_slots_len: u32 = bincode::Decode::decode(d)?;
            if touched_slots_len > 0 {
                let mut touched_storage_slots = Vec::with_capacity(touched_slots_len as usize);
                for _ in 0..touched_slots_len {
                    let slot: [u8; 32] = bincode::Decode::decode(d)?;
                    touched_storage_slots.push(U256::from_le_bytes(slot));
                }
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
                let mut transfers = Vec::with_capacity(transfers_len as usize);
                for _ in 0..transfers_len {
                    let recipient: [u8; 20] = bincode::Decode::decode(d)?;
                    let amount: [u8; 32] = bincode::Decode::decode(d)?;
                    transfers.push((Address::from(recipient), U256::from_le_bytes(amount)));
                }
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
    fn test_runtime_new_frame_input_v1_encode_decode() {
        let mut storage = BTreeMap::new();
        let mut v = RuntimeNewFrameInputV1 {
            metadata: [1, 2, 3].into(),
            input: [4, 5, 6, 7].into(),
            context: [8, 9, 10, 11, 12].into(),
            storage: Some(storage.clone()),
            balance: None,
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
            balance: None,
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
