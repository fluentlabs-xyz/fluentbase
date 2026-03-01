use crate::Bytes;
pub use ::bincode::{
    config::{Config, Configuration, Fixint, LittleEndian},
    de::{read::Reader, Decoder},
    error::DecodeError,
    *,
};
use core::ops::{Deref, DerefMut};

#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ZeroCopyBytes(pub Bytes);

impl ZeroCopyBytes {
    pub fn new() -> Self {
        Self(Bytes::new())
    }
}

impl From<Bytes> for ZeroCopyBytes {
    fn from(value: Bytes) -> Self {
        ZeroCopyBytes(value)
    }
}
impl From<ZeroCopyBytes> for Bytes {
    fn from(value: ZeroCopyBytes) -> Self {
        value.0
    }
}

impl Deref for ZeroCopyBytes {
    type Target = Bytes;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for ZeroCopyBytes {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl AsRef<[u8]> for ZeroCopyBytes {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

pub struct BytesReader {
    bytes: Bytes,
}

impl BytesReader {
    pub fn new(bytes: Bytes) -> Self {
        Self { bytes }
    }
}

/// Extension methods for a Reader that is backed by `Bytes`.
pub trait BytesReaderExt: Reader {
    /// Take the next `n` bytes as a zero-copy `Bytes` slice and advance the reader.
    fn take_bytes(&mut self, n: usize) -> Result<Bytes, DecodeError>;
}

impl BytesReaderExt for BytesReader {
    #[inline]
    fn take_bytes(&mut self, n: usize) -> Result<Bytes, DecodeError> {
        if n > self.bytes.len() {
            return Err(DecodeError::UnexpectedEnd {
                additional: n - self.bytes.len(),
            });
        }
        let out = self.bytes.slice(..n);
        self.bytes = self.bytes.slice(n..);
        Ok(out)
    }
}

impl Reader for BytesReader {
    #[inline(always)]
    fn read(&mut self, bytes: &mut [u8]) -> Result<(), DecodeError> {
        if bytes.len() > self.bytes.len() {
            return Err(DecodeError::UnexpectedEnd {
                additional: bytes.len() - self.bytes.len(),
            });
        }
        let (read_slice, _remaining) = self.bytes.split_at(bytes.len());
        bytes.copy_from_slice(read_slice);
        self.bytes = self.bytes.slice(bytes.len()..);
        Ok(())
    }

    #[inline]
    fn peek_read(&mut self, n: usize) -> Option<&[u8]> {
        self.bytes.get(..n)
    }

    #[inline]
    fn consume(&mut self, n: usize) {
        self.bytes = self.bytes.slice(n..);
    }
}

pub trait DecodeBytes<Context>: Sized {
    /// Attempt to decode this type with the given [bincode::Decode].
    fn decode_bytes<D: Decoder<Context = Context, R = BytesReader>>(
        decoder: &mut D,
    ) -> Result<Self, DecodeError>;
}

impl<Context> DecodeBytes<Context> for ZeroCopyBytes {
    fn decode_bytes<D>(decoder: &mut D) -> Result<Self, DecodeError>
    where
        D: Decoder<Context = Context, R = BytesReader>,
    {
        let len_u64 = u64::decode(decoder)?;
        let len: usize = len_u64
            .try_into()
            .map_err(|_| DecodeError::OutsideUsizeRange(len_u64))?;

        decoder.claim_container_read::<u8>(len)?;

        // Now we *know* the reader is BytesReader, so we can zero-copy:
        let bytes = decoder.reader().take_bytes(len)?;
        Ok(ZeroCopyBytes(bytes))
    }
}
impl<Context> DecodeBytes<Context> for Option<ZeroCopyBytes> {
    fn decode_bytes<D>(decoder: &mut D) -> Result<Self, DecodeError>
    where
        D: Decoder<Context = Context, R = BytesReader>,
    {
        let variant = match u8::decode(decoder)? {
            0 => Ok(None),
            1 => Ok(Some(ZeroCopyBytes::new())),
            x => Err(DecodeError::UnexpectedVariant {
                found: x as u32,
                allowed: &error::AllowedEnumVariants::Range { max: 1, min: 0 },
                type_name: core::any::type_name::<Option<ZeroCopyBytes>>(),
            }),
        }?;
        match variant {
            Some(_) => {
                let val = ZeroCopyBytes::decode_bytes(decoder)?;
                Ok(Some(val))
            }
            None => Ok(None),
        }
    }
}

pub fn decode_from_bytes<D: DecodeBytes<()>, C: Config>(
    src: Bytes,
    config: C,
) -> Result<(D, usize), DecodeError> {
    let original_len_bytes = src.len();
    let reader = BytesReader::new(src);
    let mut decoder = de::DecoderImpl::<_, C, ()>::new(reader, config, ());
    let result = D::decode_bytes(&mut decoder)?;
    let bytes_read = original_len_bytes - decoder.reader().bytes.len();
    Ok((result, bytes_read))
}
