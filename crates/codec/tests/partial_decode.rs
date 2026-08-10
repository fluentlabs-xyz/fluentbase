//! `Encoder::partial_decode` against what `Encoder::encode` wrote.
//!
//! There is no external oracle for this: `partial_decode` is our own API, with no counterpart in
//! alloy or in the Solidity ABI specification. The only thing it can be checked against is the
//! encoder, which is why this suite is separate from `abi_conformance.rs` - that file's charter is
//! our bytes against alloy's, and mixing an internal API into it would muddle both.
//!
//! It was written because mutation testing measured the gap: of 170 mutants in `partial_decode`
//! across the crate, 111 survived the whole suite - a 65% survival rate against 22% for `encode`.
//! The function is public (`SolidityABI::partial_decode`, `encoder.rs:93`) and almost nothing
//! checked its result.
//!
//! # The contract
//!
//! The doc comment on the trait says the return is `(data_offset, data_length)`, which is true but
//! means three different things:
//!
//! 1. **A static type** reports the extent of its field: `(offset, align_up::<ALIGN>(HEADER_SIZE))`.
//!    Nothing is read from the buffer. `offset` must be echoed back - reporting a width while
//!    ignoring where the field starts is what made `Option<T>` disagree with itself.
//!
//! 2. **A dynamic type in Solidity mode** reports where the *length word* sits, not where the
//!    payload does: `data_offset` is the offset word read at `offset`, and `data_length` is the
//!    word read at `data_offset`. The payload therefore begins at `data_offset + 32`. For `Bytes`
//!    and `String` the length counts bytes; for `Vec<T>` it counts elements.
//!
//! 3. **A dynamic type in compact mode** reports the payload directly: it begins at `data_offset`,
//!    with no length word in front of it.
//!
//! # Where this suite deliberately stops
//!
//! Solidity-mode offset words are written as absolute buffer positions, and
//! `read_bytes_header_solidity` reads them back the same way. The specification requires them to be
//! relative to the start of the enclosing encoding, and the two coincide only while that encoding
//! starts at buffer offset 0 - a known limitation, still open. So the cases here are top-level
//! values and
//! direct members of a top-level composite, where the contract is defined. Asserting anything about
//! deeper nesting would pin behaviour we already know to be wrong.
//!
//! `HashMap` and `HashSet` are also left out on purpose: their wire format is an extension of ours
//! that has never been written down, and pinning it with tests before specifying it would only make
//! the undocumented behaviour harder to change. Spec first.

use alloy_primitives::{Address, Bytes, FixedBytes, I256, U256};
use byteorder::{BigEndian, ByteOrder, LittleEndian};
use bytes::BytesMut;
use fluentbase_codec::{align_up, Encoder};

// ---------------------------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------------------------

#[derive(Default)]
struct Report {
    failures: Vec<String>,
    checked: usize,
}

impl Report {
    fn fail(&mut self, case: &str, detail: String) {
        self.failures.push(format!("  {case}\n      {detail}"));
    }

    fn check(&mut self, case: &str, actual: (usize, usize), expected: (usize, usize)) {
        self.checked += 1;
        if actual != expected {
            self.fail(
                case,
                format!("returned {actual:?}, the contract says {expected:?}"),
            );
        }
    }

    /// Panics with every failure at once - a suite that reports only its first hides how wide a
    /// problem is.
    fn assert_clean(&self, suite: &str) {
        if self.failures.is_empty() {
            println!("{suite}: {} checks, all hold", self.checked);
            return;
        }
        panic!(
            "\n{suite}: {} of {} checks broke the contract\n\n{}\n",
            self.failures.len(),
            self.checked,
            self.failures.join("\n")
        );
    }
}

/// Reads one aligned word as the codec writes it.
fn word_at<B: ByteOrder, const ALIGN: usize>(buf: &[u8], position: usize) -> usize {
    let width = align_up::<ALIGN>(4);
    let slot = &buf[position..position + width];
    // Big-endian words are right-aligned in their slot, little-endian ones left-aligned.
    let bytes: [u8; 4] = if core::any::type_name::<B>().contains("Big") {
        slot[width - 4..].try_into().expect("four bytes")
    } else {
        slot[..4].try_into().expect("four bytes")
    };
    if core::any::type_name::<B>().contains("Big") {
        u32::from_be_bytes(bytes) as usize
    } else {
        u32::from_le_bytes(bytes) as usize
    }
}

/// A static type reports its own field, at the offset it was asked about and nowhere else.
fn check_static<B, const ALIGN: usize, const SOL: bool, T>(
    report: &mut Report,
    label: &str,
    value: &T,
) where
    B: ByteOrder,
    T: Encoder<B, ALIGN, SOL, false> + core::fmt::Debug,
{
    assert!(
        !<T as Encoder<B, ALIGN, SOL, false>>::IS_DYNAMIC,
        "{label} is dynamic, use check_dynamic"
    );

    let width = align_up::<ALIGN>(<T as Encoder<B, ALIGN, SOL, false>>::HEADER_SIZE);

    // Offset zero hides a `partial_decode` that ignores its offset entirely, so sweep past it.
    for offset in [0, width, width * 3] {
        let mut buf = BytesMut::new();
        if let Err(error) = value.encode(&mut buf, offset) {
            report.fail(
                &format!("{label} at {offset}"),
                format!("encode: {error:?}"),
            );
            continue;
        }
        match <T as Encoder<B, ALIGN, SOL, false>>::partial_decode(&buf, offset) {
            Ok(actual) => report.check(&format!("{label} at {offset}"), actual, (offset, width)),
            Err(error) => report.fail(
                &format!("{label} at {offset}"),
                format!("partial_decode: {error:?}"),
            ),
        }
    }
}

/// A dynamic type in Solidity mode points at its length word; the payload follows one word later.
fn check_dynamic_solidity<const ALIGN: usize, T>(
    report: &mut Report,
    label: &str,
    value: &T,
    expected_length: usize,
    payload: Option<&[u8]>,
) where
    T: Encoder<BigEndian, ALIGN, true, false> + core::fmt::Debug,
{
    let mut buf = BytesMut::new();
    if let Err(error) = value.encode(&mut buf, 0) {
        report.fail(label, format!("encode: {error:?}"));
        return;
    }
    let raw = buf.to_vec();

    let (data_offset, data_length) =
        match <T as Encoder<BigEndian, ALIGN, true, false>>::partial_decode(&buf, 0) {
            Ok(header) => header,
            Err(error) => {
                report.fail(label, format!("partial_decode: {error:?}"));
                return;
            }
        };

    report.checked += 1;
    if data_length != expected_length {
        report.fail(
            label,
            format!("length {data_length}, the value has {expected_length}"),
        );
    }

    report.checked += 1;
    let length_word = word_at::<BigEndian, ALIGN>(&raw, data_offset);
    if length_word != data_length {
        report.fail(
            label,
            format!("the word at data_offset {data_offset} is {length_word}, not the reported length {data_length}"),
        );
    }

    if let Some(expected) = payload {
        report.checked += 1;
        let start = data_offset + 32;
        let end = start + expected.len();
        if raw.len() < end {
            report.fail(
                label,
                format!("payload would run to {end}, the buffer is {}", raw.len()),
            );
        } else if &raw[start..end] != expected {
            report.fail(
                label,
                format!("payload at {start} is {:?}", &raw[start..end]),
            );
        }
    }
}

/// A dynamic type in compact mode points straight at its payload.
fn check_dynamic_compact<const ALIGN: usize, T>(
    report: &mut Report,
    label: &str,
    value: &T,
    payload: Option<&[u8]>,
) where
    T: Encoder<LittleEndian, ALIGN, false, false> + core::fmt::Debug,
{
    let mut buf = BytesMut::new();
    if let Err(error) = value.encode(&mut buf, 0) {
        report.fail(label, format!("encode: {error:?}"));
        return;
    }
    let raw = buf.to_vec();

    let (data_offset, data_length) =
        match <T as Encoder<LittleEndian, ALIGN, false, false>>::partial_decode(&buf, 0) {
            Ok(header) => header,
            Err(error) => {
                report.fail(label, format!("partial_decode: {error:?}"));
                return;
            }
        };

    report.checked += 1;
    if data_offset + data_length > raw.len() {
        report.fail(
            label,
            format!(
                "the region {data_offset}..{} runs past the {}-byte encoding",
                data_offset + data_length,
                raw.len()
            ),
        );
    }

    if let Some(expected) = payload {
        report.checked += 1;
        let end = data_offset + expected.len();
        if raw.len() < end {
            report.fail(
                label,
                format!("payload would run to {end}, the buffer is {}", raw.len()),
            );
        } else if &raw[data_offset..end] != expected {
            report.fail(
                label,
                format!("payload at {data_offset} is {:?}", &raw[data_offset..end]),
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Static types
// ---------------------------------------------------------------------------------------------

macro_rules! statics {
    ($report:expr, $B:ty, $align:literal, $sol:literal, $($label:literal => $value:expr),+ $(,)?) => {
        $( check_static::<$B, $align, $sol, _>($report, $label, &$value); )+
    };
}

#[test]
fn static_types_report_their_field_in_solidity_mode() {
    let mut r = Report::default();

    statics!(&mut r, BigEndian, 32, true,
        "bool"          => true,
        "u8"            => 7u8,
        "u16"           => 7u16,
        "u32"           => 7u32,
        "u64"           => 7u64,
        "u128"          => 7u128,
        "i8"            => -7i8,
        "i16"           => -7i16,
        "i32"           => -7i32,
        "i64"           => -7i64,
        "i128"          => -7i128,
        "U256"          => U256::from(7),
        "I256"          => I256::try_from(-7).unwrap(),
        "Address"       => Address::repeat_byte(3),
        "FixedBytes<1>" => FixedBytes::<1>::from([1u8]),
        "FixedBytes<11>"=> FixedBytes::<11>::from([2u8; 11]),
        "FixedBytes<32>"=> FixedBytes::<32>::from([3u8; 32]),
    );

    r.assert_clean("static types, Solidity mode");
}

#[test]
fn static_types_report_their_field_in_compact_mode() {
    let mut r = Report::default();

    statics!(&mut r, LittleEndian, 4, false,
        "bool"          => true,
        "u8"            => 7u8,
        "u16"           => 7u16,
        "u32"           => 7u32,
        "u64"           => 7u64,
        "u128"          => 7u128,
        "i8"            => -7i8,
        "i16"           => -7i16,
        "i32"           => -7i32,
        "i64"           => -7i64,
        "i128"          => -7i128,
        "U256"          => U256::from(7),
        "I256"          => I256::try_from(-7).unwrap(),
        "Address"       => Address::repeat_byte(3),
        "FixedBytes<1>" => FixedBytes::<1>::from([1u8]),
        "FixedBytes<11>"=> FixedBytes::<11>::from([2u8; 11]),
        "FixedBytes<32>"=> FixedBytes::<32>::from([3u8; 32]),
    );

    r.assert_clean("static types, compact mode");
}

/// `Option<T>` must report one width whichever variant it holds, or a reader stepping over a field
/// lands in a different place depending on the value it just skipped.
#[test]
fn an_optional_field_reports_one_width_whichever_variant_it_holds() {
    let mut r = Report::default();

    check_static::<BigEndian, 32, true, _>(&mut r, "Option<u32>, Some", &Some(7u32));
    check_static::<BigEndian, 32, true, _>(&mut r, "Option<u32>, None", &Option::<u32>::None);
    check_static::<LittleEndian, 4, false, _>(&mut r, "Option<u32>, Some, compact", &Some(7u32));
    check_static::<LittleEndian, 4, false, _>(
        &mut r,
        "Option<u32>, None, compact",
        &Option::<u32>::None,
    );

    r.assert_clean("optional fields");
}

/// A fixed array is one static field as wide as all its elements together.
#[test]
fn a_fixed_array_reports_the_whole_run_of_elements() {
    let mut r = Report::default();

    check_static::<BigEndian, 32, true, _>(&mut r, "[u64; 3]", &[1u64, 2, 3]);
    check_static::<BigEndian, 32, true, _>(&mut r, "[U256; 2]", &[U256::from(1), U256::from(2)]);
    check_static::<LittleEndian, 4, false, _>(&mut r, "[u64; 3], compact", &[1u64, 2, 3]);

    r.assert_clean("fixed arrays");
}

/// A fixed array of a *dynamic* type is itself dynamic: its head is an offset, and what it reports
/// is where the body starts and how wide that body's head area is - one word per element, not a
/// length. A separate branch from the static case above, and one nothing reached before.
#[test]
fn a_fixed_array_of_a_dynamic_type_reports_its_head_area() {
    let mut r = Report::default();

    for texts in [
        [String::new(), String::new(), String::new()],
        [
            "alpha".to_string(),
            "beta".to_string(),
            "0123456789012345678901234567890123".to_string(),
        ],
    ] {
        let mut buf = BytesMut::new();
        <[String; 3] as Encoder<BigEndian, 32, true, false>>::encode(&texts, &mut buf, 0)
            .expect("encoding a fixed array of strings");

        let label = format!("[String; 3], first {:?}", texts[0]);
        match <[String; 3] as Encoder<BigEndian, 32, true, false>>::partial_decode(&buf, 0) {
            // The body starts where the head word points, and covers three offset words.
            Ok(actual) => {
                let body_offset = word_at::<BigEndian, 32>(&buf, 0);
                r.check(&label, actual, (body_offset, 32 * 3));
            }
            Err(error) => r.fail(&label, format!("partial_decode: {error:?}")),
        }
    }

    r.assert_clean("fixed arrays of dynamic elements");
}

/// The empty shapes, and the one-element tuple that forwards to its member. All three returned
/// constants that nothing looked at.
#[test]
fn empty_and_forwarding_shapes_report_what_they_occupy() {
    let mut r = Report::default();

    for offset in [0usize, 32, 96] {
        let buf = BytesMut::zeroed(offset + 32);

        // Nothing on the wire, so nothing to point at - and that is true at every offset.
        r.checked += 1;
        match <() as Encoder<BigEndian, 32, true, false>>::partial_decode(&buf, offset) {
            Ok((0, 0)) => {}
            Ok(other) => r.fail(
                &format!("() at {offset}"),
                format!("returned {other:?}, expected (0, 0)"),
            ),
            Err(error) => r.fail(&format!("() at {offset}"), format!("{error:?}")),
        }

        r.checked += 1;
        let phantom = core::marker::PhantomData::<BigEndian>;
        match <core::marker::PhantomData<BigEndian> as Encoder<BigEndian, 32, true, false>>::partial_decode(
            &buf, offset,
        ) {
            Ok((0, 0)) => {}
            Ok(other) => r.fail(
                &format!("PhantomData at {offset}"),
                format!("returned {other:?}, expected (0, 0)"),
            ),
            Err(error) => r.fail(&format!("PhantomData at {offset}"), format!("{error:?}")),
        }
        let _ = phantom;
    }

    // A one-element tuple is its member, so it must report exactly what the member reports.
    check_static::<BigEndian, 32, true, _>(&mut r, "(u32,)", &(7u32,));
    check_static::<BigEndian, 32, true, _>(&mut r, "(U256,)", &(U256::from(7),));
    check_static::<LittleEndian, 4, false, _>(&mut r, "(u32,), compact", &(7u32,));

    r.assert_clean("empty and forwarding shapes");
}

/// A header that runs past the end of the buffer is an error, not a guess. `Option<T>` is the only
/// static type that checks, and its guard was unexercised.
#[test]
fn a_truncated_buffer_is_rejected() {
    let mut r = Report::default();

    let mut buf = BytesMut::new();
    <Option<u32> as Encoder<BigEndian, 32, true, false>>::encode(&Some(7u32), &mut buf, 0)
        .expect("encoding an option");
    let full = buf.to_vec();

    for cut in [0usize, 1, 31, 32, 63] {
        r.checked += 1;
        let truncated = BytesMut::from(&full[..cut]);
        if <Option<u32> as Encoder<BigEndian, 32, true, false>>::partial_decode(&truncated, 0)
            .is_ok()
        {
            r.fail(
                &format!("Option<u32> cut to {cut}"),
                format!(
                    "accepted a buffer {cut} bytes long, the field is {} wide",
                    full.len()
                ),
            );
        }
    }

    // The whole field is there, so this one must succeed.
    r.checked += 1;
    if <Option<u32> as Encoder<BigEndian, 32, true, false>>::partial_decode(&buf, 0).is_err() {
        r.fail(
            "Option<u32>, complete",
            "rejected a complete field".to_string(),
        );
    }

    r.assert_clean("truncated buffers");
}

// ---------------------------------------------------------------------------------------------
// Dynamic types
// ---------------------------------------------------------------------------------------------

/// The lengths that have historically separated correct implementations from lucky ones: empty,
/// one, and either side of a word boundary.
const LENGTHS: [usize; 7] = [0, 1, 31, 32, 33, 64, 65];

#[test]
fn byte_strings_point_at_their_length_word_in_solidity_mode() {
    let mut r = Report::default();

    for len in LENGTHS {
        let payload: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();

        check_dynamic_solidity::<32, _>(
            &mut r,
            &format!("Bytes, len {len}"),
            &Bytes::from(payload.clone()),
            len,
            Some(&payload),
        );

        let text = "y".repeat(len);
        check_dynamic_solidity::<32, _>(
            &mut r,
            &format!("String, len {len}"),
            &text,
            len,
            Some(text.as_bytes()),
        );
    }

    r.assert_clean("byte strings, Solidity mode");
}

#[test]
fn vectors_report_their_element_count_in_solidity_mode() {
    let mut r = Report::default();

    for len in [0usize, 1, 2, 3, 7] {
        let numbers: Vec<U256> = (0..len).map(U256::from).collect();
        check_dynamic_solidity::<32, _>(
            &mut r,
            &format!("Vec<U256>, len {len}"),
            &numbers,
            len,
            None,
        );

        let texts: Vec<String> = (0..len).map(|i| "z".repeat(i * 33)).collect();
        check_dynamic_solidity::<32, _>(
            &mut r,
            &format!("Vec<String>, len {len}"),
            &texts,
            len,
            None,
        );
    }

    r.assert_clean("vectors, Solidity mode");
}

#[test]
fn byte_strings_point_at_their_payload_in_compact_mode() {
    let mut r = Report::default();

    for len in LENGTHS {
        let payload: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();

        check_dynamic_compact::<4, _>(
            &mut r,
            &format!("Bytes, len {len}"),
            &Bytes::from(payload.clone()),
            Some(&payload),
        );

        let text = "y".repeat(len);
        check_dynamic_compact::<4, _>(
            &mut r,
            &format!("String, len {len}"),
            &text,
            Some(text.as_bytes()),
        );
    }

    r.assert_clean("byte strings, compact mode");
}

#[test]
fn vectors_stay_inside_their_encoding_in_compact_mode() {
    let mut r = Report::default();

    for len in [0usize, 1, 2, 3, 7] {
        // The body of a `Vec<u32>` is its elements laid out one after another, so the region the
        // header points at must be exactly those bytes. Reading the header from the wrong word
        // returns the element count as the offset, which this catches.
        let numbers: Vec<u32> = (0..len as u32).collect();
        let body: Vec<u8> = numbers.iter().flat_map(|n| n.to_le_bytes()).collect();
        check_dynamic_compact::<4, _>(
            &mut r,
            &format!("Vec<u32>, len {len}"),
            &numbers,
            Some(&body),
        );

        let texts: Vec<String> = (0..len).map(|i| "z".repeat(i * 5)).collect();
        check_dynamic_compact::<4, _>(&mut r, &format!("Vec<String>, len {len}"), &texts, None);
    }

    r.assert_clean("vectors, compact mode");
}
