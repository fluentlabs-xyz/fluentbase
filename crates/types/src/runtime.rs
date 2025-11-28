use crate::ExitCode;
use alloc::vec::Vec;
use alloy_primitives::{Address, Bytes, B256, U256};
use bincode::de::read::Reader;
use bincode::de::Decoder;
use hashbrown::HashMap;

fn encode_opt_hashmap<E: bincode::enc::Encoder, K, V, const K_SIZE: usize, const V_SIZE: usize>(
    e: &mut E,
    hm: &Option<HashMap<K, V>>,
) -> Result<(), bincode::error::EncodeError> {
    let len = hm.as_ref().map_or(0, |map| map.len() as u32);
    let presented: bool = len > 0;
    bincode::Encode::encode(&presented, e)?;
    if presented {
        if let Some(hm) = &hm {
            bincode::Encode::encode(&len, e)?;
            for (k, v) in hm {
                let k_slice = unsafe { &*(k as *const K as *const [u8; K_SIZE]) };
                bincode::Encode::encode(k_slice, e)?;
                let v_slice = unsafe { &*(v as *const V as *const [u8; V_SIZE]) };
                bincode::Encode::encode(v_slice, e)?;
            }
        }
    }
    Ok(())
}

fn decode_opt_hashmap<C, D: Decoder<Context = C>, K, V, const K_SIZE: usize, const V_SIZE: usize>(
    d: &mut D,
) -> Result<Option<HashMap<K, V>>, bincode::error::DecodeError>
where
    K: Eq + core::hash::Hash + Copy + Sized,
    V: Copy + Sized,
{
    let presented: bool = bincode::Decode::decode(d)?;
    if presented {
        let storage_len: u32 = bincode::Decode::decode(d)?;
        let mut hm = HashMap::<K, V>::with_capacity(storage_len as usize);
        for _ in 0..storage_len {
            let mut k_restored = [0u8; K_SIZE];
            let mut v_restored = [0u8; V_SIZE];
            d.reader().read(&mut k_restored)?;
            let k = unsafe { *(&k_restored as *const [u8; K_SIZE] as *const K) };
            d.reader().read(&mut v_restored)?;
            let v = unsafe { *(&v_restored as *const [u8; V_SIZE] as *const V) };
            hm.insert(k, v);
        }
        Ok(Some(hm))
    } else {
        Ok(None)
    }
}

fn encode_logs<E: bincode::enc::Encoder>(
    e: &mut E,
    logs: &Vec<(Vec<B256>, Bytes)>,
) -> Result<(), bincode::error::EncodeError> {
    let len = logs.len() as u32;
    let presented: bool = len > 0;
    bincode::Encode::encode(&presented, e)?;
    if presented {
        bincode::Encode::encode(&len, e)?;
        for (topics, data) in logs {
            bincode::Encode::encode(data.as_ref(), e)?;
            bincode::Encode::encode(&(topics.len() as u32), e)?;
            for topic in topics {
                let topic_slice =
                    unsafe { &*(topic as *const B256 as *const [u8; B256::len_bytes()]) };
                bincode::Encode::encode(topic_slice, e)?;
            }
        }
    }
    Ok(())
}

fn decode_logs<C, D: Decoder<Context = C>>(
    d: &mut D,
) -> Result<Vec<(Vec<B256>, Bytes)>, bincode::error::DecodeError> {
    let presented: bool = bincode::Decode::decode(d)?;
    let mut logs = Vec::<(Vec<B256>, Bytes)>::new();
    if presented {
        let len: u32 = bincode::Decode::decode(d)?;
        logs.reserve_exact(len as usize);
        for _ in 0..len {
            let data: Vec<u8> = bincode::Decode::decode(d)?;
            let topics_len: u32 = bincode::Decode::decode(d)?;
            let mut topics = Vec::<B256>::with_capacity(topics_len as usize);
            for _ in 0..topics_len {
                let mut topic_restored = [0u8; B256::len_bytes()];
                d.reader().read(&mut topic_restored)?;
                let topic: &B256 =
                    unsafe { &*(&topic_restored as *const [u8; B256::len_bytes()] as *const B256) };
                topics.push(*topic);
            }
            logs.push((topics, data.into()));
        }
    };
    Ok(logs)
}

#[derive(Default, Clone, Debug, PartialEq)]
pub struct RuntimeNewFrameInputV1 {
    pub metadata: Bytes,
    pub input: Bytes,
    pub storage: Option<HashMap<U256, U256>>,
    pub balances: Option<HashMap<Address, U256>>,
}

impl bincode::Encode for RuntimeNewFrameInputV1 {
    fn encode<E: bincode::enc::Encoder>(
        &self,
        e: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        bincode::Encode::encode(self.metadata.as_ref(), e)?;
        bincode::Encode::encode(&self.input.as_ref(), e)?;

        encode_opt_hashmap::<_, _, _, { U256::BYTES }, { U256::BYTES }>(e, &self.storage)?;

        encode_opt_hashmap::<_, _, _, { Address::len_bytes() }, { U256::BYTES }>(
            e,
            &self.balances,
        )?;

        Ok(())
    }
}

impl<C> bincode::Decode<C> for RuntimeNewFrameInputV1 {
    fn decode<D: Decoder<Context = C>>(d: &mut D) -> Result<Self, bincode::error::DecodeError> {
        let metadata: Vec<u8> = bincode::Decode::decode(d)?;
        let input: Vec<u8> = bincode::Decode::decode(d)?;

        let storage: Option<HashMap<U256, U256>> =
            decode_opt_hashmap::<_, _, _, _, { U256::BYTES }, { U256::BYTES }>(d)?;

        let balances: Option<HashMap<Address, U256>> =
            decode_opt_hashmap::<_, _, _, _, { Address::len_bytes() }, { U256::BYTES }>(d)?;

        Ok(Self {
            metadata: metadata.into(),
            input: input.into(),
            storage,
            balances,
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
    fn decode<D: Decoder<Context = C>>(d: &mut D) -> Result<Self, bincode::error::DecodeError> {
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
    pub logs: Vec<(Vec<B256>, Bytes)>,
}
impl bincode::Encode for RuntimeExecutionOutcomeV1 {
    fn encode<E: bincode::enc::Encoder>(
        &self,
        e: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        bincode::Encode::encode(self.output.as_ref(), e)?;

        encode_opt_hashmap::<_, _, _, { U256::BYTES }, { U256::BYTES }>(e, &self.storage)?;

        encode_logs(e, &self.logs)?;

        Ok(())
    }
}

impl<C> bincode::Decode<C> for RuntimeExecutionOutcomeV1 {
    fn decode<D: Decoder<Context = C>>(d: &mut D) -> Result<Self, bincode::error::DecodeError> {
        let output: Vec<u8> = bincode::Decode::decode(d)?;

        let storage = decode_opt_hashmap::<_, _, _, _, { U256::BYTES }, { U256::BYTES }>(d)?;

        let logs = decode_logs(d)?;

        Ok(Self {
            output: output.into(),
            storage,
            logs,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::bincode_helpers::{decode, encode};
    use crate::{RuntimeExecutionOutcomeV1, RuntimeNewFrameInputV1};
    use alloy_primitives::{Address, Bytes};
    use alloy_primitives::{B256, U256};
    use hashbrown::HashMap;

    #[test]
    fn test_runtime_new_frame_input_v1_encode_decode() {
        let mut storage = HashMap::new();
        let mut balances = HashMap::new();
        let mut v = RuntimeNewFrameInputV1 {
            metadata: [1, 2, 3].into(),
            input: [4, 5, 6, 7].into(),
            storage: Some(storage.clone()),
            balances: Some(balances.clone()),
        };
        let v_encoded = encode(&v).unwrap();
        let (v_decoded, read_count) = decode::<RuntimeNewFrameInputV1>(&v_encoded).unwrap();
        assert_eq!(v_encoded.len(), read_count);
        v.storage = None;
        v.balances = None;
        assert_eq!(v_decoded, v);

        storage.insert(U256::from_le_bytes([1; 32]), U256::from_le_bytes([2; 32]));
        storage.insert(U256::from_le_bytes([3; 32]), U256::from_le_bytes([4; 32]));
        balances.insert(Address::with_last_byte(8), U256::from_le_bytes([2; 32]));
        balances.insert(Address::with_last_byte(5), U256::from_le_bytes([4; 32]));
        let v = RuntimeNewFrameInputV1 {
            metadata: [1, 2, 3].into(),
            input: [4, 5, 6, 7].into(),
            storage: Some(storage.clone()),
            balances: Some(balances.clone()),
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
            logs: events.clone(),
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
            logs: events,
        };
        let v_encoded = encode(&v).unwrap();
        let (v_decoded, read_count) = decode::<RuntimeExecutionOutcomeV1>(&v_encoded).unwrap();
        assert_eq!(v_encoded.len(), read_count);
        assert_eq!(v_decoded, v);
    }
}
