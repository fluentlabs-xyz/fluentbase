//! Decoders must turn hostile calldata into `CodecError`, never into a bounds panic.
//!
//! Every case below feeds a decoder an offset or length that points outside the encoded body.
//! Before the checked-range helpers were threaded through the decoders, each of these sliced
//! `chunk()` directly and aborted the whole execution instead of failing the call.

use alloy_primitives::{Address, Bytes, FixedBytes, U256};
use bytes::BytesMut;
use fluentbase_codec::{
    encoder::{CompactABI, SolidityABI},
    Codec, CodecError,
};
use proptest::prelude::*;

/// A dynamic struct shaped like the derived WebAuthn payloads that reach `decode` from calldata.
#[derive(Codec, Default, Debug, PartialEq)]
struct DynamicStruct {
    flag: bool,
    payload: Bytes,
    words: Vec<u32>,
}

/// A struct with no dynamic field, so the generated decoder takes the static-head branch.
#[derive(Codec, Default, Debug, PartialEq)]
struct StaticStruct {
    a: u32,
    b: u32,
}

fn sample_dynamic_struct() -> DynamicStruct {
    DynamicStruct {
        flag: true,
        payload: Bytes::from(vec![1, 2, 3, 4, 5]),
        words: vec![10, 20, 30],
    }
}

/// Overwrites the 32-byte word at `word_index` with `value`, right-aligned as Solidity encodes it.
fn set_word(buf: &mut BytesMut, word_index: usize, value: u32) {
    let end = (word_index + 1) * 32;
    buf[end - 4..end].copy_from_slice(&value.to_be_bytes());
}

#[test]
fn dynamic_struct_head_offset_past_end_is_an_error() {
    let mut buf = BytesMut::new();
    SolidityABI::encode(&sample_dynamic_struct(), &mut buf, 0).unwrap();

    // The head word points the struct body far beyond the encoded input.
    set_word(&mut buf, 0, u32::MAX);
    let encoded = buf.freeze();

    let error = SolidityABI::<DynamicStruct>::decode(&encoded, 0)
        .expect_err("an out-of-range struct body offset must be rejected");
    assert!(matches!(error, CodecError::Decoding(_)));
}

#[test]
fn dynamic_struct_head_offset_just_past_end_is_an_error() {
    let mut buf = BytesMut::new();
    SolidityABI::encode(&sample_dynamic_struct(), &mut buf, 0).unwrap();
    let len = buf.len() as u32;

    // One byte past the last readable byte: the off-by-one that a `len`-only check would miss.
    set_word(&mut buf, 0, len + 1);
    let encoded = buf.freeze();

    SolidityABI::<DynamicStruct>::decode(&encoded, 0)
        .expect_err("a struct body offset one byte past the end must be rejected");
}

#[test]
fn static_struct_head_past_end_is_an_error() {
    let mut buf = BytesMut::new();
    SolidityABI::encode(&StaticStruct { a: 1, b: 2 }, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    let error = SolidityABI::<StaticStruct>::decode(&encoded, encoded.len() + 32)
        .expect_err("a static head past the end of the input must be rejected");
    assert!(matches!(error, CodecError::Decoding(_)));
}

#[test]
fn tuple_dynamic_offset_past_end_is_an_error() {
    let mut buf = BytesMut::new();
    SolidityABI::encode(&(Bytes::from(vec![1, 2, 3]),), &mut buf, 0).unwrap();

    set_word(&mut buf, 0, u32::MAX);
    let encoded = buf.freeze();

    let error = SolidityABI::<(Bytes,)>::decode(&encoded, 0)
        .expect_err("an out-of-range tuple body offset must be rejected");
    assert!(matches!(error, CodecError::Decoding(_)));
}

#[test]
fn multi_field_tuple_dynamic_offset_past_end_is_an_error() {
    let mut buf = BytesMut::new();
    SolidityABI::encode(&(Bytes::from(vec![1, 2, 3]), U256::from(7)), &mut buf, 0).unwrap();

    set_word(&mut buf, 0, u32::MAX);
    let encoded = buf.freeze();

    SolidityABI::<(Bytes, U256)>::decode(&encoded, 0)
        .expect_err("an out-of-range tuple body offset must be rejected");
}

#[test]
fn bytes_body_length_past_end_is_an_error() {
    let mut buf = BytesMut::new();
    SolidityABI::encode(&Bytes::from(vec![1, 2, 3, 4, 5]), &mut buf, 0).unwrap();

    // Keep the offset word valid but claim a body far longer than the input.
    set_word(&mut buf, 1, u32::MAX);
    let encoded = buf.freeze();

    let error = SolidityABI::<Bytes>::decode(&encoded, 0)
        .expect_err("a bytes length past the end of the input must be rejected");
    assert!(matches!(error, CodecError::Decoding(_)));
}

#[test]
fn truncated_input_is_an_error_for_every_decoder() {
    let mut buf = BytesMut::new();
    SolidityABI::encode(&sample_dynamic_struct(), &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    // Every proper prefix of a valid encoding must fail cleanly rather than panic.
    for len in 0..encoded.len() {
        let truncated = encoded.slice(..len);
        SolidityABI::<DynamicStruct>::decode(&truncated, 0)
            .expect_err("a truncated encoding must be rejected");
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Arbitrary bytes must never panic a decoder, whatever they decode to.
    #[test]
    fn arbitrary_bytes_never_panic_solidity(input in proptest::collection::vec(any::<u8>(), 0..512)) {
        let buf = Bytes::from(input);

        let _ = SolidityABI::<DynamicStruct>::decode(&buf, 0);
        let _ = SolidityABI::<StaticStruct>::decode(&buf, 0);
        let _ = SolidityABI::<(Bytes, U256)>::decode(&buf, 0);
        let _ = SolidityABI::<(Bytes,)>::decode(&buf, 0);
        let _ = SolidityABI::<Bytes>::decode(&buf, 0);
        let _ = SolidityABI::<Vec<u32>>::decode(&buf, 0);
        let _ = SolidityABI::<Vec<Bytes>>::decode(&buf, 0);
        let _ = SolidityABI::<Address>::decode(&buf, 0);
        let _ = SolidityABI::<U256>::decode(&buf, 0);
        let _ = SolidityABI::<FixedBytes<32>>::decode(&buf, 0);
    }

    /// The same guarantee for the compact (WASM) mode decoders.
    #[test]
    fn arbitrary_bytes_never_panic_compact(input in proptest::collection::vec(any::<u8>(), 0..512)) {
        let buf = Bytes::from(input);

        let _ = CompactABI::<DynamicStruct>::decode(&buf, 0);
        let _ = CompactABI::<StaticStruct>::decode(&buf, 0);
        let _ = CompactABI::<Bytes>::decode(&buf, 0);
        let _ = CompactABI::<Vec<u32>>::decode(&buf, 0);
        let _ = CompactABI::<Vec<Bytes>>::decode(&buf, 0);
        let _ = CompactABI::<Address>::decode(&buf, 0);
        let _ = CompactABI::<U256>::decode(&buf, 0);
    }

    /// Decoding at an arbitrary offset into an arbitrary buffer must not panic either.
    #[test]
    fn arbitrary_offsets_never_panic(
        input in proptest::collection::vec(any::<u8>(), 0..256),
        offset in 0usize..4096,
    ) {
        let buf = Bytes::from(input);

        let _ = SolidityABI::<DynamicStruct>::decode(&buf, offset);
        let _ = SolidityABI::<StaticStruct>::decode(&buf, offset);
        let _ = SolidityABI::<Bytes>::decode(&buf, offset);
        let _ = CompactABI::<DynamicStruct>::decode(&buf, offset);
        let _ = CompactABI::<Vec<u32>>::decode(&buf, offset);
    }

    /// Corrupting a single word of a valid encoding is the realistic attack shape: the surrounding
    /// words stay well-formed, so the decoder gets as deep as possible before hitting the bad range.
    #[test]
    fn single_word_corruption_never_panics(word_index in 0usize..8, value in any::<u32>()) {
        let mut buf = BytesMut::new();
        SolidityABI::encode(&sample_dynamic_struct(), &mut buf, 0).unwrap();

        if (word_index + 1) * 32 <= buf.len() {
            set_word(&mut buf, word_index, value);
            let encoded = buf.freeze();
            let _ = SolidityABI::<DynamicStruct>::decode(&encoded, 0);
        }
    }

    /// Valid encodings still round-trip unchanged.
    #[test]
    fn valid_encodings_round_trip(
        flag in any::<bool>(),
        payload in proptest::collection::vec(any::<u8>(), 0..96),
        words in proptest::collection::vec(any::<u32>(), 0..16),
    ) {
        let value = DynamicStruct {
            flag,
            payload: Bytes::from(payload),
            words,
        };

        let mut buf = BytesMut::new();
        SolidityABI::encode(&value, &mut buf, 0).unwrap();
        let encoded = buf.freeze();

        let decoded = SolidityABI::<DynamicStruct>::decode(&encoded, 0)
            .expect("a value we just encoded must decode");
        prop_assert_eq!(decoded, value);
    }
}
