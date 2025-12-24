use crate::ExitCode;
use alloc::vec::Vec;
use alloy_primitives::Bytes;
use bincode::de::Decoder;

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
            exit_code: exit_code.into(),
        })
    }
}
