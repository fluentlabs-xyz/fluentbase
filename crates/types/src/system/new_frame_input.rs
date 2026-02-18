use crate::{system::JournalLog, ExitCode};
use alloc::vec::Vec;
use alloy_primitives::{Bytes, U256};
use bincode::de::Decoder;
use hashbrown::HashMap;

#[derive(Default, Clone, Debug, PartialEq)]
pub struct RuntimeNewFrameInputV1 {
    pub metadata: Bytes,
    pub input: Bytes,
    pub context: Bytes,
    pub storage: Option<HashMap<U256, U256>>,
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
        Ok(())
    }
}

impl<C> bincode::Decode<C> for RuntimeNewFrameInputV1 {
    fn decode<D: Decoder<Context = C>>(d: &mut D) -> Result<Self, bincode::error::DecodeError> {
        let metadata: Vec<u8> = bincode::Decode::decode(d)?;
        let input: Vec<u8> = bincode::Decode::decode(d)?;
        let context: Vec<u8> = bincode::Decode::decode(d)?;
        let storage_len: u32 = bincode::Decode::decode(d)?;
        let storage = if storage_len > 0 {
            let mut storage = HashMap::<U256, U256>::with_capacity(storage_len as usize);
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
        Ok(Self {
            metadata: metadata.into(),
            input: input.into(),
            context: context.into(),
            storage,
        })
    }
}

#[derive(Default, Clone, Debug, PartialEq)]
pub struct RuntimeExecutionOutcomeV1 {
    pub exit_code: ExitCode,
    pub output: Bytes,
    pub storage: Option<HashMap<U256, U256>>,
    pub logs: Vec<JournalLog>,
    pub new_metadata: Option<Bytes>,
}

impl RuntimeExecutionOutcomeV1 {
    pub fn encode(&self) -> Vec<u8> {
        bincode::encode_to_vec(self, bincode::config::legacy()).unwrap()
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let (result, _bytes_read) =
            bincode::decode_from_slice(bytes, bincode::config::legacy()).ok()?;
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
        Ok(())
    }
}

impl<C> bincode::Decode<C> for RuntimeExecutionOutcomeV1 {
    fn decode<D: Decoder<Context = C>>(d: &mut D) -> Result<Self, bincode::error::DecodeError> {
        let exit_code: i32 = bincode::Decode::decode(d)?;
        let output: Vec<u8> = bincode::Decode::decode(d)?;
        let storage_len: u32 = bincode::Decode::decode(d)?;
        let storage = if storage_len > 0 {
            let mut storage = HashMap::<U256, U256>::with_capacity(storage_len as usize);
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
        let new_metadata: Option<Vec<u8>> = bincode::Decode::decode(d)?;
        Ok(Self {
            exit_code: ExitCode::from(exit_code),
            output: output.into(),
            storage,
            logs,
            new_metadata: new_metadata.map(Into::into),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        bincode_helpers::{decode, encode},
        system::{
            new_frame_input::{RuntimeExecutionOutcomeV1, RuntimeNewFrameInputV1},
            JournalLog,
        },
        ExitCode,
    };
    use alloy_primitives::{bytes, B256, U256};
    use hashbrown::HashMap;

    #[test]
    fn test_runtime_new_frame_input_v1_encode_decode() {
        let mut storage = HashMap::new();
        let mut v = RuntimeNewFrameInputV1 {
            metadata: [1, 2, 3].into(),
            input: [4, 5, 6, 7].into(),
            context: [8, 9, 10, 11, 12].into(),
            storage: Some(storage.clone()),
        };
        let v_encoded = encode(&v).unwrap();
        let (v_decoded, read_count) = decode::<RuntimeNewFrameInputV1>(&v_encoded).unwrap();
        assert_eq!(v_encoded.len(), read_count);
        v.storage = None;
        assert_eq!(v_decoded, v);

        storage.insert(U256::from_le_bytes([1; 32]), U256::from_le_bytes([2; 32]));
        storage.insert(U256::from_le_bytes([3; 32]), U256::from_le_bytes([4; 32]));
        let v = RuntimeNewFrameInputV1 {
            metadata: [1, 2, 3].into(),
            input: [4, 5, 6, 7].into(),
            context: [8, 9, 10, 11, 12].into(),
            storage: Some(storage.clone()),
        };
        let v_encoded = encode(&v).unwrap();
        let (v_decoded, read_count) = decode::<RuntimeNewFrameInputV1>(&v_encoded).unwrap();
        assert_eq!(v_encoded.len(), read_count);
        assert_eq!(v_decoded, v);
    }

    #[test]
    fn test_encode_decode_none_metadata() {
        let v = RuntimeExecutionOutcomeV1 {
            exit_code: ExitCode::BadConversionToInteger,
            output: bytes!("112233"),
            storage: None,
            logs: vec![],
            new_metadata: None,
        };
        let v_encoded = v.encode();
        let v_decoded = RuntimeExecutionOutcomeV1::decode(&v_encoded).unwrap();
        assert_eq!(v_decoded, v);
    }

    #[test]
    fn test_runtime_output_v1_encode_decode() {
        let mut storage = HashMap::new();
        let mut logs = Vec::new();
        let mut v = RuntimeExecutionOutcomeV1 {
            exit_code: ExitCode::PrecompileError,
            output: [1, 2, 3].into(),
            storage: Some(storage.clone()),
            logs: logs.clone(),
            new_metadata: Some(bytes!("112233")),
        };
        let v_encoded = encode(&v).unwrap();
        let (v_decoded, read_count) = decode::<RuntimeExecutionOutcomeV1>(&v_encoded).unwrap();
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
        };
        let v_encoded = encode(&v).unwrap();
        let (v_decoded, read_count) = decode::<RuntimeExecutionOutcomeV1>(&v_encoded).unwrap();
        assert_eq!(v_encoded.len(), read_count);
        assert_eq!(v_decoded, v);
    }
}
