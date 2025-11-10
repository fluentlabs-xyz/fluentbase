use crate::bincode_helpers::VecWriter;
use crate::{ExitCode, B256};
use alloc::string::String;
use alloc::vec::Vec;
use core::ops::Range;
use revm_helpers::reusable_pool::global::{VecU8, VEC_U8_REUSABLE_POOL_CAPACITY};

#[derive(Clone, Default, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SyscallInvocationParams {
    pub code_hash: B256,
    pub input: Range<usize>,
    pub fuel_limit: u64,
    pub state: u32,
    pub fuel16_ptr: u32,
}

impl SyscallInvocationParams {
    pub fn encode(&self) -> Vec<u8> {
        bincode::encode_to_vec(self, bincode::config::legacy()).unwrap()
    }

    pub fn encode_into(&self, dst: &mut VecU8) {
        bincode::encode_into_writer(
            self,
            VecWriter::new(dst.inner_mut()),
            bincode::config::legacy(),
        )
        .unwrap();
        if dst.inner_ref().capacity() > VEC_U8_REUSABLE_POOL_CAPACITY {
            panic!("reallocation occur")
        }
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let (result, _bytes_read) =
            bincode::decode_from_slice(bytes, bincode::config::legacy()).ok()?;
        Some(result)
    }
}

impl ::bincode::Encode for SyscallInvocationParams {
    fn encode<__E: bincode::enc::Encoder>(
        &self,
        encoder: &mut __E,
    ) -> Result<(), bincode::error::EncodeError> {
        ::bincode::Encode::encode(&self.code_hash.0, encoder)?;
        ::bincode::Encode::encode(&self.fuel_limit, encoder)?;
        ::bincode::Encode::encode(&self.state, encoder)?;
        ::bincode::Encode::encode(&self.fuel16_ptr, encoder)?;
        ::bincode::Encode::encode(&self.input, encoder)?;
        Ok(())
    }
}

impl<__Context> ::bincode::Decode<__Context> for SyscallInvocationParams {
    fn decode<__D: ::bincode::de::Decoder<Context = __Context>>(
        decoder: &mut __D,
    ) -> Result<Self, bincode::error::DecodeError> {
        let code_hash: [u8; 32] = bincode::Decode::decode(decoder)?;
        let fuel_limit: u64 = bincode::Decode::decode(decoder)?;
        let state: u32 = bincode::Decode::decode(decoder)?;
        let fuel16_ptr: u32 = bincode::Decode::decode(decoder)?;
        let input: Range<usize> = bincode::Decode::decode(decoder)?;
        Ok(Self {
            code_hash: B256::from(code_hash),
            input: input.into(),
            fuel_limit,
            state,
            fuel16_ptr,
        })
    }
}

#[derive(Debug)]
pub struct SyscallResult<T> {
    pub data: T,
    pub fuel_consumed: u64,
    pub fuel_refunded: i64,
    pub status: ExitCode,
}

impl SyscallResult<()> {
    pub fn is_ok<I: Into<ExitCode>>(status: I) -> bool {
        Into::<ExitCode>::into(status) == ExitCode::Ok
    }
    pub fn is_panic<I: Into<ExitCode>>(status: I) -> bool {
        Into::<ExitCode>::into(status) == ExitCode::Panic
    }
    pub fn is_err<I: Into<ExitCode>>(status: I) -> bool {
        Into::<ExitCode>::into(status) == ExitCode::Err
    }
}

impl<T> SyscallResult<T> {
    pub fn new<I: Into<ExitCode>>(
        data: T,
        fuel_consumed: u64,
        fuel_refunded: i64,
        status: I,
    ) -> Self {
        Self {
            data,
            fuel_consumed,
            fuel_refunded,
            status: Into::<ExitCode>::into(status),
        }
    }
    pub fn from_old<U>(old: SyscallResult<T>, data: U) -> SyscallResult<U> {
        SyscallResult {
            data,
            fuel_consumed: old.fuel_consumed,
            fuel_refunded: old.fuel_refunded,
            status: old.status,
        }
    }
    pub fn from_old_empty(old: SyscallResult<T>) -> SyscallResult<()> {
        SyscallResult::from_old(old, ())
    }
    pub fn expect<I: Into<String>>(self, msg: I) -> Self {
        if !self.status.is_ok() {
            panic!("syscall failed with status {}: {}", self.status, msg.into());
        }
        self
    }
    pub fn unwrap(self) -> T {
        if !self.status.is_ok() {
            panic!("syscall failed with status ({})", self.status);
        }
        self.data
    }
}

impl<T: Default> SyscallResult<T> {
    pub fn unwrap_or_default(self) -> T {
        if self.status.is_ok() {
            self.data
        } else {
            T::default()
        }
    }

    pub fn map<U: Default>(
        self,
        f: impl FnOnce(SyscallResult<T>) -> SyscallResult<U>,
    ) -> SyscallResult<U> {
        f(self)
    }
}

impl<T> core::ops::Deref for SyscallResult<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}
impl<T> core::ops::DerefMut for SyscallResult<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}
