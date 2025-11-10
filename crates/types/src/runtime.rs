use alloc::vec::Vec;
use alloy_primitives::Bytes;

#[derive(Default, Clone, Debug, PartialEq)]
pub struct RuntimeNewFrameInputV1 {
    pub metadata: Vec<u8>,
    pub input: Vec<u8>,
}

impl Drop for RuntimeNewFrameInputV1 {
    fn drop(&mut self) {
        revm_helpers::reusable_pool::global::vec_u8_reusable_pool::take_recycle(&mut self.metadata);
        revm_helpers::reusable_pool::global::vec_u8_reusable_pool::take_recycle(&mut self.input);
    }
}

impl bincode::Encode for RuntimeNewFrameInputV1 {
    fn encode<E: bincode::enc::Encoder>(
        &self,
        e: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        bincode::Encode::encode(&self.metadata, e)?;
        bincode::Encode::encode(&self.input, e)?;
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
