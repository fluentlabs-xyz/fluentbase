use alloc::vec::Vec;
use alloy_primitives::Bytes;
use bincode::de::read::Reader;
use bincode::de::Decoder;
#[cfg(not(feature = "std"))]
use revm_helpers::reusable_pool::global::VecU8;

#[derive(Default, Clone, Debug, PartialEq)]
pub struct RuntimeNewFrameInputV1 {
    #[cfg(feature = "std")]
    pub metadata: Bytes,
    #[cfg(not(feature = "std"))]
    pub metadata: VecU8,
    #[cfg(feature = "std")]
    pub input: Bytes,
    #[cfg(not(feature = "std"))]
    pub input: VecU8,
}

impl bincode::Encode for RuntimeNewFrameInputV1 {
    fn encode<E: bincode::enc::Encoder>(
        &self,
        e: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        bincode::Encode::encode(&self.metadata.as_ref(), e)?;
        bincode::Encode::encode(&self.input.as_ref(), e)?;
        Ok(())
    }
}

#[cfg(feature = "std")]
fn try_decode_slice<C, D: bincode::de::Decoder<Context = C>>(
    d: &mut D,
) -> Result<Bytes, bincode::error::DecodeError> {
    let mut result = Vec::new();

    let mut len_or_cap_buf = [0u8; 4];

    d.reader().read(&mut len_or_cap_buf)?;
    let len = u32::from_le_bytes(len_or_cap_buf) as usize;
    // skip cap
    d.reader().read(&mut len_or_cap_buf)?;
    if len > 0 {
        let data_slice = d.reader().peek_read(len).unwrap();
        result.extend_from_slice(&data_slice);
        d.reader().consume(len);
    }

    Ok(result.into())
}

#[cfg(not(feature = "std"))]
fn try_decode_slice<C, D: bincode::de::Decoder<Context = C>>(
    d: &mut D,
) -> Result<VecU8, bincode::error::DecodeError> {
    let mut result = VecU8::default_for_reuse();

    let mut len_or_cap_buf = [0u8; 4];

    d.reader().read(&mut len_or_cap_buf)?;
    let len = u32::from_le_bytes(len_or_cap_buf) as usize;
    // skip cap
    d.reader().read(&mut len_or_cap_buf)?;
    if len > 0 {
        let data_slice = d.reader().peek_read(len).unwrap();
        result.extend_from_slice(&data_slice);
        d.reader().consume(len);
    }

    Ok(result)
}

impl<C> bincode::Decode<C> for RuntimeNewFrameInputV1 {
    fn decode<D: bincode::de::Decoder<Context = C>>(
        d: &mut D,
    ) -> Result<Self, bincode::error::DecodeError> {
        let metadata = try_decode_slice(d)?;
        let input = try_decode_slice(d)?;

        Ok(Self { metadata, input })
    }
}

#[derive(Default, Clone, Debug, PartialEq)]
pub struct RuntimeInterruptionOutcomeV1 {
    #[cfg(feature = "std")]
    pub output: Bytes,
    #[cfg(not(feature = "std"))]
    pub output: VecU8,
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
        let output = try_decode_slice(d)?;

        let fuel_consumed: u64 = bincode::Decode::decode(d)?;
        let fuel_refunded: i64 = bincode::Decode::decode(d)?;
        let exit_code: i32 = bincode::Decode::decode(d)?;
        Ok(Self {
            output,
            fuel_consumed,
            fuel_refunded,
            exit_code,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::bincode_helpers::VecWriter;
    use crate::{RuntimeInterruptionOutcomeV1, RuntimeNewFrameInputV1};
    use alloy_primitives::Bytes;
    #[cfg(not(feature = "std"))]
    use revm_helpers::reusable_pool::global::VecU8;

    #[test]
    fn enc_dec_runtime_new_frame_input_v1() {
        let original = RuntimeNewFrameInputV1 {
            metadata: [1, 2, 3].into(),
            input: [4, 5, 6, 7, 8, 9].into(),
        };
        let mut buffer = Vec::new();
        let writer = VecWriter::new(&mut buffer);
        bincode::encode_into_writer(&original, writer, bincode::config::legacy()).unwrap();
        let (v2_decoded, _): (RuntimeNewFrameInputV1, _) =
            bincode::decode_from_slice(&buffer, bincode::config::legacy()).unwrap();
        assert_eq!(original, v2_decoded);
    }

    #[test]
    fn enc_dec_runtime_interruption_outcome_v1() {
        let original = RuntimeInterruptionOutcomeV1 {
            output: [1, 2, 3, 4, 5].into(),
            fuel_consumed: 1,
            fuel_refunded: 2,
            exit_code: 3,
        };
        let mut buffer = Vec::new();
        let writer = VecWriter::new(&mut buffer);
        bincode::encode_into_writer(&original, writer, bincode::config::legacy()).unwrap();
        let (decoded, _): (RuntimeInterruptionOutcomeV1, _) =
            bincode::decode_from_slice(&buffer, bincode::config::legacy()).unwrap();
        assert_eq!(original, decoded);
    }
}
