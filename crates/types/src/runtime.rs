use alloc::vec::Vec;
use alloy_primitives::Bytes;
use bincode::error::AllowedEnumVariants;

#[derive(Default, Clone, Debug, PartialEq)]
pub struct RuntimeNewFrameInputV1 {
    pub metadata: Bytes,
    pub input: Bytes,
}

impl bincode::Encode for RuntimeNewFrameInputV1 {
    fn encode<E: bincode::enc::Encoder>(
        &self,
        e: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        bincode::Encode::encode(self.metadata.as_ref(), e)?;
        bincode::Encode::encode(&self.input.as_ref(), e)?;
        Ok(())
    }
}

impl<C> bincode::Decode<C> for RuntimeNewFrameInputV1 {
    fn decode<D: bincode::de::Decoder<Context = C>>(
        d: &mut D,
    ) -> Result<Self, bincode::error::DecodeError> {
        let metadata: Vec<u8> = bincode::Decode::decode(d)?;
        let input: Vec<u8> = bincode::Decode::decode(d)?;
        Ok(Self {
            metadata: metadata.into(),
            input: input.into(),
        })
    }
}

#[derive(Default, Clone, Debug, PartialEq)]
pub struct RuntimeInterruptionOutcomeV1 {
    pub output: Bytes,
    pub fuel_consumed: u64,
    pub fuel_refunded: i64,
    pub exit_code: i32,
}

impl bincode::Encode for RuntimeInterruptionOutcomeV1 {
    fn encode<E: bincode::enc::Encoder>(
        &self,
        e: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        bincode::Encode::encode(self.output.as_ref(), e)?;
        bincode::Encode::encode(&self.fuel_consumed, e)?;
        bincode::Encode::encode(&self.fuel_refunded, e)?;
        bincode::Encode::encode(&self.exit_code, e)?;
        Ok(())
    }
}

impl<C> bincode::Decode<C> for RuntimeInterruptionOutcomeV1 {
    fn decode<D: bincode::de::Decoder<Context = C>>(
        d: &mut D,
    ) -> Result<Self, bincode::error::DecodeError> {
        let output: Vec<u8> = bincode::Decode::decode(d)?;
        let fuel_consumed: u64 = bincode::Decode::decode(d)?;
        let fuel_refunded: i64 = bincode::Decode::decode(d)?;
        let exit_code: i32 = bincode::Decode::decode(d)?;
        Ok(Self {
            output: output.into(),
            fuel_consumed,
            fuel_refunded,
            exit_code,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RuntimeInputOutputV1 {
    RuntimeNewFrameInputV1(RuntimeNewFrameInputV1),
    RuntimeInterruptionOutcomeV1(RuntimeInterruptionOutcomeV1),
}

impl RuntimeInputOutputV1 {
    pub fn encode(&self) -> Vec<u8> {
        bincode::encode_to_vec(&self, bincode::config::legacy()).unwrap()
    }

    pub fn decode(input: &[u8]) -> Self {
        let (result, _) = bincode::decode_from_slice(input, bincode::config::legacy()).unwrap();
        result
    }
}

impl bincode::Encode for RuntimeInputOutputV1 {
    fn encode<E: bincode::enc::Encoder>(
        &self,
        e: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        match self {
            RuntimeInputOutputV1::RuntimeNewFrameInputV1(value) => {
                bincode::Encode::encode(&0x01u8, e)?;
                bincode::Encode::encode(value, e)?;
            }
            RuntimeInputOutputV1::RuntimeInterruptionOutcomeV1(value) => {
                bincode::Encode::encode(&0x02u8, e)?;
                bincode::Encode::encode(value, e)?;
            }
        }
        Ok(())
    }
}

impl<C> bincode::Decode<C> for RuntimeInputOutputV1 {
    fn decode<D: bincode::de::Decoder<Context = C>>(
        d: &mut D,
    ) -> Result<Self, bincode::error::DecodeError> {
        let type_version: u8 = bincode::Decode::decode(d)?;
        Ok(match type_version {
            0x01 => Self::RuntimeNewFrameInputV1(bincode::Decode::decode(d)?),
            0x02 => Self::RuntimeInterruptionOutcomeV1(bincode::Decode::decode(d)?),
            _ => {
                return Err(bincode::error::DecodeError::UnexpectedVariant {
                    type_name: "unknown RuntimeInputOutputV1 variant",
                    allowed: &AllowedEnumVariants::Allowed(&[0x01, 0x02]),
                    found: 0,
                })
            }
        })
    }
}
