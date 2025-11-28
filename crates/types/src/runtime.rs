use crate::bincode_helpers::{decode, encode};
use crate::ExitCode;
use alloc::vec::Vec;
use alloy_primitives::{Bytes, B256, U256};
use bincode::de::read::Reader;
use bincode::de::Decoder;
use hashbrown::HashMap;

#[derive(Default, Clone, Debug, PartialEq)]
pub struct RuntimeNewFrameInputV1 {
    pub metadata: Bytes,
    pub input: Bytes,
    pub storage: Option<HashMap<U256, U256>>,
    // pub balance: Option<HashMap<[u8; 32], [u8; 32]>>,
}

impl bincode::Encode for RuntimeNewFrameInputV1 {
    fn encode<E: bincode::enc::Encoder>(
        &self,
        e: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        bincode::Encode::encode(self.metadata.as_ref(), e)?;
        bincode::Encode::encode(&self.input.as_ref(), e)?;

        let storage_len = self.storage.as_ref().map_or(0, |map| map.len() as u32);
        let storage_presented = self.storage.is_some() && storage_len > 0;
        bincode::Encode::encode(&storage_presented, e)?;
        if storage_len > 0 {
            if let Some(storage) = &self.storage {
                bincode::Encode::encode(&storage_len, e)?;
                for (k, v) in storage {
                    let k_slice = unsafe { &*(k as *const U256 as *const [u8; 32]) };
                    bincode::Encode::encode(k_slice, e)?;
                    let v_slice = unsafe { &*(v as *const U256 as *const [u8; 32]) };
                    bincode::Encode::encode(v_slice, e)?;
                }
            }
        }
        Ok(())
    }
}

impl<C> bincode::Decode<C> for RuntimeNewFrameInputV1 {
    fn decode<D: Decoder<Context = C>>(d: &mut D) -> Result<Self, bincode::error::DecodeError> {
        let metadata: Vec<u8> = bincode::Decode::decode(d)?;
        let input: Vec<u8> = bincode::Decode::decode(d)?;
        let storage_presented: bool = bincode::Decode::decode(d)?;
        let storage = if storage_presented {
            let storage_len: u32 = bincode::Decode::decode(d)?;
            let mut storage = HashMap::<U256, U256>::with_capacity(storage_len as usize);
            for _ in 0..storage_len {
                let mut k_restored: [u8; 32] = [0u8; 32];
                let mut v_restored: [u8; 32] = [0u8; 32];
                d.reader().read(&mut k_restored)?;
                d.reader().read(&mut v_restored)?;
                let k = unsafe { *(&k_restored as *const [u8; 32] as *const U256) };
                let v = unsafe { *(&v_restored as *const [u8; 32] as *const U256) };
                storage.insert(k, v);
            }
            Some(storage)
        } else {
            None
        };
        Ok(Self {
            metadata: metadata.into(),
            input: input.into(),
            storage,
        })
    }
}

#[derive(Default, Clone, Debug, PartialEq)]
pub struct RuntimeInterruptionOutcomeV1 {
    pub halted_frame: bool,
    pub output: Bytes,
    pub fuel_consumed: u64,
    pub fuel_refunded: i64,
    pub exit_code: ExitCode,
}

impl bincode::Encode for RuntimeInterruptionOutcomeV1 {
    fn encode<E: bincode::enc::Encoder>(
        &self,
        e: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        bincode::Encode::encode(&self.halted_frame, e)?;
        bincode::Encode::encode(self.output.as_ref(), e)?;
        bincode::Encode::encode(&self.fuel_consumed, e)?;
        bincode::Encode::encode(&self.fuel_refunded, e)?;
        let exit_code = self.exit_code.into_i32();
        bincode::Encode::encode(&exit_code, e)?;
        Ok(())
    }
}

impl<C> bincode::Decode<C> for RuntimeInterruptionOutcomeV1 {
    fn decode<D: bincode::de::Decoder<Context = C>>(
        d: &mut D,
    ) -> Result<Self, bincode::error::DecodeError> {
        let halted_frame: bool = bincode::Decode::decode(d)?;
        let output: Vec<u8> = bincode::Decode::decode(d)?;
        let fuel_consumed: u64 = bincode::Decode::decode(d)?;
        let fuel_refunded: i64 = bincode::Decode::decode(d)?;
        let exit_code: i32 = bincode::Decode::decode(d)?;
        Ok(Self {
            halted_frame,
            output: output.into(),
            fuel_consumed,
            fuel_refunded,
            exit_code: ExitCode::from(exit_code),
        })
    }
}

#[derive(Default, Clone, Debug, PartialEq)]
pub struct RuntimeExecutionOutcomeV1 {
    pub output: Bytes,
    pub storage: Option<HashMap<U256, U256>>,
    pub events: Vec<(Vec<B256>, Bytes)>,
}
impl bincode::Encode for RuntimeExecutionOutcomeV1 {
    fn encode<E: bincode::enc::Encoder>(
        &self,
        e: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        bincode::Encode::encode(self.output.as_ref(), e)?;

        let storage_len = self.storage.as_ref().map_or(0, |map| map.len() as u32);
        let storage_presented = self.storage.is_some() && storage_len > 0;
        bincode::Encode::encode(&storage_presented, e)?;
        if storage_presented {
            if let Some(storage) = &self.storage {
                bincode::Encode::encode(&storage_len, e)?;
                for (k, v) in storage {
                    let k_slice = unsafe { &*(k as *const U256 as *const [u8; 32]) };
                    bincode::Encode::encode(k_slice, e)?;
                    let v_slice = unsafe { &*(v as *const U256 as *const [u8; 32]) };
                    bincode::Encode::encode(v_slice, e)?;
                }
            }
        }

        let events_len = self.events.len() as u32;
        let events_presented = events_len > 0;
        bincode::Encode::encode(&events_presented, e)?;
        if events_presented {
            bincode::Encode::encode(&events_len, e)?;
            for (topics, data) in &self.events {
                bincode::Encode::encode(data.as_ref(), e)?;
                bincode::Encode::encode(&(topics.len() as u32), e)?;
                for topic in topics {
                    let topic_slice = unsafe { &*(topic as *const B256 as *const [u8; 32]) };
                    bincode::Encode::encode(topic_slice, e)?;
                }
            }
        }

        Ok(())
    }
}

impl<C> bincode::Decode<C> for RuntimeExecutionOutcomeV1 {
    fn decode<D: Decoder<Context = C>>(d: &mut D) -> Result<Self, bincode::error::DecodeError> {
        let output: Vec<u8> = bincode::Decode::decode(d)?;

        let storage_presented: bool = bincode::Decode::decode(d)?;
        let storage = if storage_presented {
            let len: u32 = bincode::Decode::decode(d)?;
            let mut storage = HashMap::<U256, U256>::with_capacity(len as usize);
            for _ in 0..len {
                let mut k_restored: [u8; 32] = [0u8; 32];
                let mut v_restored: [u8; 32] = [0u8; 32];
                d.reader().read(&mut k_restored)?;
                d.reader().read(&mut v_restored)?;
                let k = unsafe { *(&k_restored as *const [u8; 32] as *const U256) };
                let v = unsafe { *(&v_restored as *const [u8; 32] as *const U256) };
                storage.insert(k, v);
            }
            Some(storage)
        } else {
            None
        };

        let events_presented: bool = bincode::Decode::decode(d)?;
        let mut events = Vec::<(Vec<B256>, Bytes)>::new();
        if events_presented {
            let events_len: u32 = bincode::Decode::decode(d)?;
            events.reserve_exact(events_len as usize);
            for _ in 0..events_len {
                let data: Vec<u8> = bincode::Decode::decode(d)?;
                let topics_len: u32 = bincode::Decode::decode(d)?;
                let mut topics = Vec::<B256>::with_capacity(topics_len as usize);
                for _ in 0..topics_len {
                    let mut topic_restored: [u8; 32] = [0u8; 32];
                    d.reader().read(&mut topic_restored)?;
                    let topic: &B256 =
                        unsafe { &*(&topic_restored as *const [u8; 32] as *const B256) };
                    topics.push(*topic);
                }
                events.push((topics, data.into()));
            }
        };

        Ok(Self {
            output: output.into(),
            storage,
            events,
        })
    }
}

#[test]
fn test_runtime_new_frame_input_v1_encode_decode() {
    let mut storage = HashMap::new();
    let mut v = RuntimeNewFrameInputV1 {
        metadata: [1, 2, 3].into(),
        input: [4, 5, 6, 7].into(),
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
        storage: Some(storage.clone()),
    };
    let v_encoded = encode(&v).unwrap();
    let (v_decoded, read_count) = decode::<RuntimeNewFrameInputV1>(&v_encoded).unwrap();
    assert_eq!(v_encoded.len(), read_count);
    assert_eq!(v_decoded, v);
}

#[test]
fn test_runtime_output_v1_encode_decode() {
    let mut storage = HashMap::new();
    let mut events = Vec::<(Vec<B256>, Bytes)>::new();
    let mut v = RuntimeExecutionOutcomeV1 {
        output: [1, 2, 3].into(),
        storage: Some(storage.clone()),
        events: events.clone(),
    };
    let v_encoded = encode(&v).unwrap();
    let (v_decoded, read_count) = decode::<RuntimeExecutionOutcomeV1>(&v_encoded).unwrap();
    assert_eq!(v_encoded.len(), read_count);
    v.storage = None;
    assert_eq!(v_decoded, v);

    storage.insert(U256::from_le_bytes([1; 32]), U256::from_le_bytes([2; 32]));
    storage.insert(U256::from_le_bytes([3; 32]), U256::from_le_bytes([4; 32]));
    events.push((
        [B256::repeat_byte(4), B256::repeat_byte(7)].into(),
        [].into(),
    ));
    events.push(([].into(), [4, 5].into()));
    events.push((
        [B256::repeat_byte(87), B256::repeat_byte(23)].into(),
        [4, 5].into(),
    ));
    let v = RuntimeExecutionOutcomeV1 {
        output: [1, 2, 3].into(),
        storage: Some(storage.clone()),
        events,
    };
    let v_encoded = encode(&v).unwrap();
    let (v_decoded, read_count) = decode::<RuntimeExecutionOutcomeV1>(&v_encoded).unwrap();
    assert_eq!(v_encoded.len(), read_count);
    assert_eq!(v_decoded, v);
}
