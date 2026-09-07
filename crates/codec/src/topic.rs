//! Solidity's encoding of indexed event parameters.
//!
//! An indexed parameter of a value type occupies its topic word directly. Reference types --
//! `string`, `bytes`, arrays (both fixed and dynamic) and structs -- do not fit in a word, so the
//! topic is `keccak256` of a preimage that is *not* ordinary ABI encoding: it carries no length
//! prefix and no head/tail offsets, and every member sits in place, padded to a whole number of
//! words.
//!
//! Hashing is left to the caller, because contracts reach keccak256 through a host function; this
//! module only builds the bytes to hash. See the [Solidity ABI specification][spec].
//!
//! [spec]: https://docs.soliditylang.org/en/latest/abi-spec.html#encoding-of-indexed-event-parameters

use crate::{
    encoder::Encoder,
    error::{CodecError, EncodingError},
};
use alloc::{format, string::String, vec::Vec};
use alloy_primitives::{Address, Bytes, FixedBytes, Signed, Uint};
use byteorder::BE;
use bytes::BytesMut;

/// Width of an ABI word, and therefore of a topic.
const WORD: usize = 32;

/// Solidity's indexed-event encoding, which departs from ordinary ABI encoding for every type
/// that is not a value type.
pub trait SolidityEventTopic {
    /// `true` for reference types, whose topic is `keccak256` of the encoding produced here;
    /// `false` for value types, whose encoding *is* the topic word.
    const IS_REFERENCE_TYPE: bool;

    /// Appends this value's encoding as a member of an array or struct.
    ///
    /// Members always occupy a whole number of words, so this is the encoding Solidity calls the
    /// topic preimage.
    fn encode_topic_preimage(&self, out: &mut BytesMut) -> Result<(), CodecError>;

    /// Appends the bytes whose `keccak256` is the topic of a top-level indexed parameter -- or,
    /// for a value type, the topic word itself.
    ///
    /// Identical to [`Self::encode_topic_preimage`] except for `bytes` and `string`, which are
    /// hashed over their raw contents when indexed directly.
    fn encode_topic_input(&self, out: &mut BytesMut) -> Result<(), CodecError> {
        self.encode_topic_preimage(out)
    }
}

/// The topic contributed by a single indexed event parameter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexedTopic {
    /// A value type: this word is the topic.
    Word([u8; WORD]),
    /// A reference type: the topic is `keccak256` of these bytes.
    Preimage(BytesMut),
}

/// Encodes one indexed event parameter the way Solidity does.
///
/// Reference types come back as a preimage rather than a topic because the caller owns the choice
/// of keccak256 implementation.
pub fn encode_indexed_topic<T: SolidityEventTopic>(value: &T) -> Result<IndexedTopic, CodecError> {
    let mut out = BytesMut::new();
    value.encode_topic_input(&mut out)?;

    if T::IS_REFERENCE_TYPE {
        return Ok(IndexedTopic::Preimage(out));
    }

    let word: [u8; WORD] = out.as_ref().try_into().map_err(|_| {
        CodecError::Encoding(EncodingError::InvalidInputData(format!(
            "an indexed value type must encode to exactly {} bytes, got {}",
            WORD,
            out.len()
        )))
    })?;

    Ok(IndexedTopic::Word(word))
}

/// Writes the 32-byte ABI word of a value type.
///
/// Solidity encodes a value type identically in a topic and in the data section, so this reuses
/// the encoder that writes the data section instead of restating the padding rules.
fn write_value_word<T>(value: &T, out: &mut BytesMut) -> Result<(), CodecError>
where
    T: Encoder<BE, WORD, true, false>,
{
    let mut word = BytesMut::new();
    value.encode(&mut word, 0)?;
    out.extend_from_slice(&word);
    Ok(())
}

/// Padding that brings a `bytes`/`string` member up to a whole number of words.
///
/// An empty member still occupies one zero word rather than disappearing.
const fn bytes_member_padding(len: usize) -> usize {
    if len == 0 {
        return WORD;
    }

    match len % WORD {
        0 => 0,
        rest => WORD - rest,
    }
}

fn write_bytes_member(value: &[u8], out: &mut BytesMut) {
    out.extend_from_slice(value);
    out.resize(out.len() + bytes_member_padding(value.len()), 0);
}

macro_rules! impl_value_type {
    ($($ty:ty),* $(,)?) => {
        $(
            impl SolidityEventTopic for $ty {
                const IS_REFERENCE_TYPE: bool = false;

                fn encode_topic_preimage(&self, out: &mut BytesMut) -> Result<(), CodecError> {
                    write_value_word(self, out)
                }
            }
        )*
    };
}

impl_value_type!(bool, u8, u16, u32, u64, i8, i16, i32, i64, u128, i128, Address);

impl<const N: usize> SolidityEventTopic for FixedBytes<N> {
    const IS_REFERENCE_TYPE: bool = false;

    fn encode_topic_preimage(&self, out: &mut BytesMut) -> Result<(), CodecError> {
        write_value_word(self, out)
    }
}

impl<const BITS: usize, const LIMBS: usize> SolidityEventTopic for Uint<BITS, LIMBS> {
    const IS_REFERENCE_TYPE: bool = false;

    fn encode_topic_preimage(&self, out: &mut BytesMut) -> Result<(), CodecError> {
        write_value_word(self, out)
    }
}

impl<const BITS: usize, const LIMBS: usize> SolidityEventTopic for Signed<BITS, LIMBS> {
    const IS_REFERENCE_TYPE: bool = false;

    fn encode_topic_preimage(&self, out: &mut BytesMut) -> Result<(), CodecError> {
        write_value_word(self, out)
    }
}

impl SolidityEventTopic for Bytes {
    const IS_REFERENCE_TYPE: bool = true;

    fn encode_topic_preimage(&self, out: &mut BytesMut) -> Result<(), CodecError> {
        write_bytes_member(self.as_ref(), out);
        Ok(())
    }

    fn encode_topic_input(&self, out: &mut BytesMut) -> Result<(), CodecError> {
        out.extend_from_slice(self.as_ref());
        Ok(())
    }
}

impl SolidityEventTopic for String {
    const IS_REFERENCE_TYPE: bool = true;

    fn encode_topic_preimage(&self, out: &mut BytesMut) -> Result<(), CodecError> {
        write_bytes_member(self.as_bytes(), out);
        Ok(())
    }

    fn encode_topic_input(&self, out: &mut BytesMut) -> Result<(), CodecError> {
        out.extend_from_slice(self.as_bytes());
        Ok(())
    }
}

/// Arrays drop their length prefix entirely and simply concatenate their members.
impl<T: SolidityEventTopic> SolidityEventTopic for Vec<T> {
    const IS_REFERENCE_TYPE: bool = true;

    fn encode_topic_preimage(&self, out: &mut BytesMut) -> Result<(), CodecError> {
        for element in self.iter() {
            element.encode_topic_preimage(out)?;
        }
        Ok(())
    }
}

impl<T: SolidityEventTopic, const N: usize> SolidityEventTopic for [T; N] {
    const IS_REFERENCE_TYPE: bool = true;

    fn encode_topic_preimage(&self, out: &mut BytesMut) -> Result<(), CodecError> {
        for element in self.iter() {
            element.encode_topic_preimage(out)?;
        }
        Ok(())
    }
}

/// Tuples follow the struct rule: their members are concatenated in place, so a nested dynamic
/// member is inlined rather than pointed at by an offset.
macro_rules! impl_tuple {
    ($($ty:ident),+) => {
        #[allow(non_snake_case)]
        impl<$($ty: SolidityEventTopic,)+> SolidityEventTopic for ($($ty,)+) {
            const IS_REFERENCE_TYPE: bool = true;

            fn encode_topic_preimage(&self, out: &mut BytesMut) -> Result<(), CodecError> {
                let ($($ty,)+) = self;
                $($ty.encode_topic_preimage(out)?;)+
                Ok(())
            }
        }
    };
}

impl SolidityEventTopic for () {
    const IS_REFERENCE_TYPE: bool = true;

    fn encode_topic_preimage(&self, _out: &mut BytesMut) -> Result<(), CodecError> {
        Ok(())
    }
}

impl_tuple!(T1);
impl_tuple!(T1, T2);
impl_tuple!(T1, T2, T3);
impl_tuple!(T1, T2, T3, T4);
impl_tuple!(T1, T2, T3, T4, T5);
impl_tuple!(T1, T2, T3, T4, T5, T6);
impl_tuple!(T1, T2, T3, T4, T5, T6, T7);
impl_tuple!(T1, T2, T3, T4, T5, T6, T7, T8);
impl_tuple!(T1, T2, T3, T4, T5, T6, T7, T8, T9);
impl_tuple!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10);
impl_tuple!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11);
impl_tuple!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12);
