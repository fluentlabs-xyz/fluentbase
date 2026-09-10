//! The decoder against input it did not produce.
//!
//! The corpus and the suite in `crates/codec` only ever feed the decoder bytes the encoder wrote.
//! That leaves the whole error-handling surface unexercised, and the offsets a decoder follows are
//! attacker-controlled: `read_bytes_header_solidity` (`crates/codec/src/bytes_codec.rs:201-210`)
//! reads `data_offset` out of the input and then reads again at that offset. Arbitrary bytes are
//! the only way to check that every such path returns an error rather than panicking.

use alloy_primitives::{Address, Bytes, FixedBytes, U256};
use codec_conformance::SolidityABI;
use proptest::prelude::*;

/// Decoding must never panic, whatever the input. A returned error is a pass; so is a value.
macro_rules! must_not_panic {
    ($raw:expr, $($ty:ty),+ $(,)?) => {{
        let raw = bytes::Bytes::from($raw);
        $(
            let _ = SolidityABI::<$ty>::decode(&raw, 0);
            let _ = SolidityABI::<$ty>::partial_decode(&raw, 0);
        )+
    }};
}

proptest! {
    // No explicit `cases`: an inline literal overrides what `Config::default()` reads from
    // PROPTEST_CASES, which silently reduced `make fuzz` to the same run as an ordinary one.
    #![proptest_config(ProptestConfig::default())]

    /// Wholly random bytes. Most are rejected early, which is the point: the guards have to hold
    /// before any arithmetic runs on the numbers read out of the buffer.
    #[test]
    fn arbitrary_bytes_are_rejected_without_panicking(
        raw in proptest::collection::vec(any::<u8>(), 0..512)
    ) {
        must_not_panic!(
            raw,
            U256, Address, bool, FixedBytes<32>,
            String, Bytes,
            Vec<U256>, Vec<String>, Vec<Bytes>, Vec<Vec<U256>>,
            [U256; 3],
            (U256, String),
            (U256, Vec<String>, Address),
        );
    }

    /// Word-aligned garbage. Random bytes rarely survive the first length check; whole words of
    /// plausible-looking offsets get further into the arithmetic.
    #[test]
    fn word_shaped_garbage_is_rejected_without_panicking(
        words in proptest::collection::vec(
            prop_oneof![
                Just(0u64),
                Just(32),
                Just(64),
                Just(0xffff_ffff),
                Just(u64::MAX),
                any::<u64>(),
            ],
            0..16,
        )
    ) {
        let mut raw = Vec::with_capacity(words.len() * 32);
        for word in words {
            raw.extend_from_slice(&[0u8; 24]);
            raw.extend_from_slice(&word.to_be_bytes());
        }
        must_not_panic!(
            raw,
            String, Bytes,
            Vec<U256>, Vec<String>, Vec<Bytes>, Vec<Vec<U256>>,
            [U256; 3],
            (U256, String),
            (U256, Vec<String>, Address),
        );
    }

    /// A well-formed encoding truncated at every boundary - the failure mode a real caller hits
    /// when calldata is cut short.
    #[test]
    fn truncated_encodings_are_rejected_without_panicking(cut in 0usize..320) {
        let value = (U256::from(7), vec!["one".to_string(), "two".to_string()], Address::ZERO);
        let mut buf = bytes::BytesMut::new();
        SolidityABI::encode(&value, &mut buf, 0).expect("encoding a well-formed value");
        let full = buf.to_vec();
        let raw = full[..cut.min(full.len())].to_vec();

        must_not_panic!(raw, (U256, Vec<String>, Address), Vec<String>, String);
    }
}
