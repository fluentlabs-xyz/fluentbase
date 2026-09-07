//! Conformance of `SolidityABI` against the Solidity ABI specification.
//!
//! `alloy-sol-types` is used as the differential oracle because it is an independent
//! implementation of the same specification - but the specification is the authority. Where the
//! two disagree, the rule being tested is quoted from
//! <https://docs.soliditylang.org/en/latest/abi-spec.html> in the case itself.
//!
//! Every case is checked in four directions, because each one fails differently:
//!
//! 1. our bytes are identical to alloy's       - we write what Solidity writes
//! 2. alloy decodes our bytes, with validation - what we write is canonical, not merely readable
//! 3. we decode alloy's bytes                  - we read what Solidity writes
//! 4. we decode our own bytes                  - the codec is self-consistent
//!
//! Direction 4 looks redundant next to 1, and is not: a codec whose encoder and decoder share a
//! wrong layout passes a round-trip while failing 1-3. Direction 1 alone is equally insufficient
//! - it says nothing about the decoder.
//!
//! Adding a shape is one `case!` line. Failures are collected rather than panicked, so a single
//! run reports every divergence in the suite instead of only the first.

use alloy_primitives::{Address, Bytes, FixedBytes, I128, I256, U128, U160, U256};

type U24 = alloy_primitives::Uint<24, 1>;
type U40 = alloy_primitives::Uint<40, 1>;
use alloy_sol_types::{sol_data, SolType};
use byteorder::BigEndian;
use bytes::BytesMut;
use fluentbase_codec::{Codec, Encoder};

// ---------------------------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------------------------

/// One failed direction, kept as text so the whole report can be printed at once.
struct Divergence {
    case: String,
    direction: &'static str,
    detail: String,
}

#[derive(Default)]
struct Report {
    divergences: Vec<Divergence>,
    checked: usize,
}

impl Report {
    fn fail(&mut self, case: &str, direction: &'static str, detail: String) {
        self.divergences.push(Divergence {
            case: case.to_string(),
            direction,
            detail,
        });
    }

    /// Panics with the full list if anything diverged.
    fn assert_clean(&self, suite: &str) {
        if self.divergences.is_empty() {
            println!("{suite}: {} cases, no divergences", self.checked);
            return;
        }

        let mut out = format!(
            "\n{suite}: {} of {} checks diverged from the Solidity ABI specification\n\n",
            self.divergences.len(),
            self.checked
        );
        for d in &self.divergences {
            out.push_str(&format!(
                "  [{}] {}\n      {}\n",
                d.direction, d.case, d.detail
            ));
        }
        panic!("{out}");
    }
}

fn our_encoding<T: Encoder<BigEndian, 32, true, false>>(value: &T) -> Vec<u8> {
    let mut buf = BytesMut::new();
    value
        .encode(&mut buf, 0)
        .expect("encoding a well-formed value must succeed");
    buf.to_vec()
}

/// Runs all four directions for one value.
///
/// `value` is the Rust-side value; `sol` is the same value in alloy's representation. They are the
/// same type for everything except derive structs, which map to a Solidity tuple.
fn check<R, S>(report: &mut Report, case: &str, value: &R, sol: &S::RustType)
where
    R: Encoder<BigEndian, 32, true, false> + PartialEq + core::fmt::Debug,
    S: SolType,
    S::RustType: PartialEq + core::fmt::Debug,
{
    report.checked += 4;

    let ours = our_encoding(value);
    let theirs = S::abi_encode(sol);

    // 1. identical bytes
    if ours != theirs {
        report.fail(
            case,
            "bytes",
            format!(
                "ours  = {}\n      alloy = {}",
                hex::encode(&ours),
                hex::encode(&theirs)
            ),
        );
    }

    // 2. alloy decodes ours, with validation: our output must be canonical, not just parseable
    match S::abi_decode_validate(&ours) {
        Ok(decoded) if &decoded == sol => {}
        Ok(decoded) => report.fail(
            case,
            "alloy reads ours",
            format!("decoded to {decoded:?}, expected {sol:?}"),
        ),
        Err(error) => report.fail(case, "alloy reads ours", format!("rejected: {error}")),
    }

    // 3. we decode alloy's
    let alloy_bytes = bytes::Bytes::from(theirs.clone());
    match <R as Encoder<BigEndian, 32, true, false>>::decode(&alloy_bytes, 0) {
        Ok(decoded) if &decoded == value => {}
        Ok(decoded) => report.fail(
            case,
            "we read alloy",
            format!("decoded to {decoded:?}, expected {value:?}"),
        ),
        Err(error) => report.fail(case, "we read alloy", format!("rejected: {error:?}")),
    }

    // 4. we decode our own
    let our_bytes = bytes::Bytes::from(ours.clone());
    match <R as Encoder<BigEndian, 32, true, false>>::decode(&our_bytes, 0) {
        Ok(decoded) if &decoded == value => {}
        Ok(decoded) => report.fail(
            case,
            "we read ours",
            format!("decoded to {decoded:?}, expected {value:?}"),
        ),
        Err(error) => report.fail(case, "we read ours", format!("rejected: {error:?}")),
    }
}

/// A case whose Rust type and alloy `RustType` are the same type.
macro_rules! case {
    ($report:expr, $sol:ty, $value:expr) => {{
        let value = $value;
        check::<_, $sol>(
            $report,
            &format!("{} = {:?}", stringify!($sol), value),
            &value,
            &value,
        );
    }};
    ($report:expr, $sol:ty, $value:expr, $label:expr) => {{
        let value = $value;
        check::<_, $sol>($report, $label, &value, &value);
    }};
}

/// A case whose Rust type differs from alloy's - derive structs against their tuple.
macro_rules! case_as {
    ($report:expr, $sol:ty, $value:expr, $sol_value:expr, $label:expr) => {{
        let value = $value;
        let sol_value = $sol_value;
        check::<_, $sol>($report, $label, &value, &sol_value);
    }};
}

// ---------------------------------------------------------------------------------------------
// Shapes used by the tables below
// ---------------------------------------------------------------------------------------------

#[derive(Codec, Default, Debug, PartialEq, Clone, Copy)]
struct StaticStruct {
    a: U256,
    b: Address,
    c: bool,
}

#[derive(Codec, Default, Debug, PartialEq, Clone)]
struct NarrowStruct {
    small: u32,
    wide: u64,
    signed: i32,
}

#[derive(Codec, Default, Debug, PartialEq, Clone)]
struct DynamicStruct {
    id: U256,
    name: String,
    blob: Bytes,
}

#[derive(Codec, Default, Debug, PartialEq, Clone)]
struct NestedStruct {
    inner: StaticStruct,
    list: Vec<U256>,
}

type SolStatic = (sol_data::Uint<256>, sol_data::Address, sol_data::Bool);
type SolNarrow = (sol_data::Uint<32>, sol_data::Uint<64>, sol_data::Int<32>);
type SolDynamic = (sol_data::Uint<256>, sol_data::String, sol_data::Bytes);
type SolNested = (SolStatic, sol_data::Array<sol_data::Uint<256>>);

fn static_struct(a: u64, b: u8, c: bool) -> (StaticStruct, (U256, Address, bool)) {
    let value = StaticStruct {
        a: U256::from(a),
        b: Address::repeat_byte(b),
        c,
    };
    let sol = (value.a, value.b, value.c);
    (value, sol)
}

fn dynamic_struct(id: u64, name: &str, blob: &[u8]) -> (DynamicStruct, (U256, String, Bytes)) {
    let value = DynamicStruct {
        id: U256::from(id),
        name: name.to_string(),
        blob: Bytes::copy_from_slice(blob),
    };
    let sol = (value.id, value.name.clone(), value.blob.clone());
    (value, sol)
}

// ---------------------------------------------------------------------------------------------
// Value types
//
// spec: "uint<M>: enc(X) is the big-endian encoding of X, padded on the higher-order (left) side
// with zero-bytes"; "int<M>: ... padded ... with 0xff bytes for negative X and with zero-bytes
// for non-negative X"; "bytes<M>: ... padded with trailing zero-bytes".
// ---------------------------------------------------------------------------------------------

#[test]
fn value_types_match_the_specification() {
    let mut r = Report::default();

    case!(&mut r, sol_data::Bool, false);
    case!(&mut r, sol_data::Bool, true);

    for v in [0u8, 1, 0x7f, 0x80, u8::MAX] {
        case!(&mut r, sol_data::Uint<8>, v);
    }
    for v in [0u16, 1, 0x7fff, 0x8000, u16::MAX] {
        case!(&mut r, sol_data::Uint<16>, v);
    }
    for v in [0u32, 1, 0x7fff_ffff, 0x8000_0000, u32::MAX] {
        case!(&mut r, sol_data::Uint<32>, v);
    }
    for v in [0u64, 1, u64::MAX / 2, u64::MAX] {
        case!(&mut r, sol_data::Uint<64>, v);
    }

    // Zero is non-negative, so it is zero-padded, not sign-extended.
    for v in [i16::MIN, -1, 0, 1, i16::MAX] {
        case!(&mut r, sol_data::Int<16>, v);
    }
    for v in [i32::MIN, -1, 0, 1, i32::MAX] {
        case!(&mut r, sol_data::Int<32>, v);
    }
    for v in [i64::MIN, -1, 0, 1, i64::MAX] {
        case!(&mut r, sol_data::Int<64>, v);
    }

    for v in [U256::ZERO, U256::from(1), U256::MAX] {
        case!(&mut r, sol_data::Uint<256>, v);
    }
    for v in [I256::MIN, I256::MINUS_ONE, I256::ZERO, I256::ONE, I256::MAX] {
        case!(&mut r, sol_data::Int<256>, v);
    }

    for v in [
        Address::ZERO,
        Address::repeat_byte(0xab),
        Address::repeat_byte(0xff),
    ] {
        case!(&mut r, sol_data::Address, v);
    }

    case!(&mut r, sol_data::FixedBytes<1>, FixedBytes::<1>::ZERO);
    case!(
        &mut r,
        sol_data::FixedBytes<1>,
        FixedBytes::<1>::new([0xff])
    );
    case!(
        &mut r,
        sol_data::FixedBytes<3>,
        FixedBytes::<3>::repeat_byte(0xab)
    );
    case!(&mut r, sol_data::FixedBytes<8>, FixedBytes::<8>::ZERO);
    case!(
        &mut r,
        sol_data::FixedBytes<16>,
        FixedBytes::<16>::repeat_byte(0xff)
    );
    case!(
        &mut r,
        sol_data::FixedBytes<31>,
        FixedBytes::<31>::repeat_byte(1)
    );
    case!(&mut r, sol_data::FixedBytes<32>, FixedBytes::<32>::ZERO);
    case!(
        &mut r,
        sol_data::FixedBytes<32>,
        FixedBytes::<32>::repeat_byte(0x5a)
    );

    r.assert_clean("value types");
}

// ---------------------------------------------------------------------------------------------
// Dynamic scalars
//
// spec: "bytes, of length k: enc(X) = enc(k) pad_right(X)"; "string: enc(X) = enc(enc_utf8(X))".
// Word-boundary lengths matter: 31, 32 and 33 exercise the padding rule.
// ---------------------------------------------------------------------------------------------

#[test]
fn dynamic_scalars_match_the_specification() {
    let mut r = Report::default();

    for len in [0usize, 1, 31, 32, 33, 64, 65] {
        case!(
            &mut r,
            sol_data::Bytes,
            Bytes::from(vec![0xab; len]),
            &format!("bytes of length {len}")
        );
        case!(
            &mut r,
            sol_data::String,
            "x".repeat(len),
            &format!("string of length {len}")
        );
    }

    case!(
        &mut r,
        sol_data::String,
        "unicode: ключ ‱ 🙂".to_string(),
        "string, multi-byte utf8"
    );

    r.assert_clean("dynamic scalars");
}

// ---------------------------------------------------------------------------------------------
// Arrays
//
// spec: "T[] where X has k elements: enc(X) = enc(k) enc((X[0], ..., X[k-1]))", i.e. the elements
// are encoded as a tuple - head/tail, with offsets relative to the start of that tuple.
//
// Lengths 0..3 are all present deliberately: a one-element container has no second head, so it
// cannot expose an offset-arithmetic bug (FLU-1112 hid behind exactly that).
// ---------------------------------------------------------------------------------------------

#[test]
fn arrays_of_static_elements_match_the_specification() {
    let mut r = Report::default();

    for len in [0usize, 1, 2, 3] {
        case!(
            &mut r,
            sol_data::Array<sol_data::Uint<256>>,
            (0..len).map(U256::from).collect::<Vec<_>>(),
            &format!("U256[] len {len}")
        );
        case!(
            &mut r,
            sol_data::Array<sol_data::Uint<32>>,
            (0..len as u32).collect::<Vec<_>>(),
            &format!("uint32[] len {len} (contains zero)")
        );
        case!(
            &mut r,
            sol_data::Array<sol_data::Address>,
            (0..len)
                .map(|i| Address::repeat_byte(i as u8))
                .collect::<Vec<_>>(),
            &format!("address[] len {len}")
        );
        case!(
            &mut r,
            sol_data::Array<sol_data::Bool>,
            (0..len).map(|i| i % 2 == 0).collect::<Vec<_>>(),
            &format!("bool[] len {len}")
        );
    }

    r.assert_clean("arrays of static elements");
}

#[test]
fn arrays_of_dynamic_elements_match_the_specification() {
    let mut r = Report::default();

    for len in [0usize, 1, 2, 3] {
        case!(
            &mut r,
            sol_data::Array<sol_data::String>,
            (0..len).map(|i| "a".repeat(i)).collect::<Vec<_>>(),
            &format!("string[] len {len} (first element empty)")
        );
        case!(
            &mut r,
            sol_data::Array<sol_data::Bytes>,
            (0..len)
                .map(|i| Bytes::from(vec![7u8; i * 16]))
                .collect::<Vec<_>>(),
            &format!("bytes[] len {len}")
        );
        case!(
            &mut r,
            sol_data::Array<sol_data::Array<sol_data::Uint<256>>>,
            (0..len)
                .map(|i| (0..i).map(U256::from).collect::<Vec<_>>())
                .collect::<Vec<_>>(),
            &format!("uint256[][] len {len}")
        );
    }

    r.assert_clean("arrays of dynamic elements");
}

#[test]
fn arrays_of_tuples_match_the_specification() {
    let mut r = Report::default();

    for len in [0usize, 1, 2, 3] {
        case!(
            &mut r,
            sol_data::Array<(sol_data::Uint<256>, sol_data::Uint<256>)>,
            (0..len)
                .map(|i| (U256::from(i), U256::from(i * 2)))
                .collect::<Vec<_>>(),
            &format!("(uint256,uint256)[] len {len} - static tuple")
        );
        case!(
            &mut r,
            sol_data::Array<(sol_data::Uint<256>, sol_data::String)>,
            (0..len)
                .map(|i| (U256::from(i), "a".repeat(i)))
                .collect::<Vec<_>>(),
            &format!("(uint256,string)[] len {len} - dynamic tuple")
        );
    }

    r.assert_clean("arrays of tuples");
}

// ---------------------------------------------------------------------------------------------
// Fixed-size arrays
//
// spec: "T[k]: enc(X) = enc((X[0], ..., X[k-1]))", and "T[k] for any dynamic T" is itself dynamic.
// ---------------------------------------------------------------------------------------------

#[test]
fn fixed_arrays_match_the_specification() {
    let mut r = Report::default();

    case!(
        &mut r,
        sol_data::FixedArray<sol_data::Uint<256>, 2>,
        [U256::ZERO, U256::from(2)]
    );
    case!(
        &mut r,
        sol_data::FixedArray<sol_data::Uint<32>, 3>,
        [0u32, 1, u32::MAX]
    );
    case!(
        &mut r,
        sol_data::FixedArray<sol_data::Address, 2>,
        [Address::ZERO, Address::repeat_byte(9)]
    );

    r.assert_clean("fixed arrays");
}

// ---------------------------------------------------------------------------------------------
// Structs
//
// spec: a struct is a tuple, so `(T1,...,Tk)` is dynamic if any Ti is dynamic, and its members
// follow the same head/tail rule.
// ---------------------------------------------------------------------------------------------

#[test]
fn structs_match_the_specification() {
    let mut r = Report::default();

    let (value, sol) = static_struct(0, 0, false);
    case_as!(&mut r, SolStatic, value, sol, "static struct, all zero");
    let (value, sol) = static_struct(5, 2, true);
    case_as!(&mut r, SolStatic, value, sol, "static struct, populated");

    let narrow = NarrowStruct {
        small: 0,
        wide: 0,
        signed: 0,
    };
    case_as!(
        &mut r,
        SolNarrow,
        narrow.clone(),
        (narrow.small, narrow.wide, narrow.signed),
        "narrow-int struct, all zero"
    );
    let narrow = NarrowStruct {
        small: 1,
        wide: u64::MAX,
        signed: -1,
    };
    case_as!(
        &mut r,
        SolNarrow,
        narrow.clone(),
        (narrow.small, narrow.wide, narrow.signed),
        "narrow-int struct, mixed signs"
    );

    let (value, sol) = dynamic_struct(0, "", &[]);
    case_as!(
        &mut r,
        SolDynamic,
        value,
        sol,
        "dynamic struct, empty members"
    );
    let (value, sol) = dynamic_struct(9, "fluent", b"abc");
    case_as!(&mut r, SolDynamic, value, sol, "dynamic struct, populated");
    let (value, sol) = dynamic_struct(1, &"n".repeat(40), &[3u8; 33]);
    case_as!(
        &mut r,
        SolDynamic,
        value,
        sol,
        "dynamic struct, members over one word"
    );

    let nested = NestedStruct {
        inner: StaticStruct {
            a: U256::from(1),
            b: Address::ZERO,
            c: false,
        },
        list: vec![U256::ZERO, U256::from(2)],
    };
    case_as!(
        &mut r,
        SolNested,
        nested.clone(),
        (
            (nested.inner.a, nested.inner.b, nested.inner.c),
            nested.list.clone()
        ),
        "struct containing a struct and an array"
    );

    r.assert_clean("structs");
}

#[test]
fn arrays_of_structs_match_the_specification() {
    let mut r = Report::default();

    for len in [0usize, 1, 2, 3] {
        let values: Vec<StaticStruct> = (0..len)
            .map(|i| static_struct(i as u64, i as u8, i % 2 == 0).0)
            .collect();
        let sols: Vec<(U256, Address, bool)> = values.iter().map(|v| (v.a, v.b, v.c)).collect();
        case_as!(
            &mut r,
            sol_data::Array<SolStatic>,
            values,
            sols,
            &format!("static struct[] len {len}")
        );

        let values: Vec<DynamicStruct> = (0..len)
            .map(|i| dynamic_struct(i as u64, &"a".repeat(i), &vec![9u8; i]).0)
            .collect();
        let sols: Vec<(U256, String, Bytes)> = values
            .iter()
            .map(|v| (v.id, v.name.clone(), v.blob.clone()))
            .collect();
        case_as!(
            &mut r,
            sol_data::Array<SolDynamic>,
            values,
            sols,
            &format!("dynamic struct[] len {len}")
        );

        // A struct whose members are all narrower than a word. Its inline width is three words
        // regardless; a stride derived from the sum of member sizes (4 + 8 + 4) is not.
        let values: Vec<NarrowStruct> = (0..len)
            .map(|i| NarrowStruct {
                small: i as u32,
                wide: i as u64 + 10,
                signed: -(i as i32) - 1,
            })
            .collect();
        let sols: Vec<(u32, u64, i32)> =
            values.iter().map(|v| (v.small, v.wide, v.signed)).collect();
        case_as!(
            &mut r,
            sol_data::Array<SolNarrow>,
            values,
            sols,
            &format!("narrow-member struct[] len {len}")
        );

        // A struct containing a struct: the inner width has to be right at both levels.
        let values: Vec<NestedStruct> = (0..len)
            .map(|i| NestedStruct {
                inner: static_struct(i as u64, i as u8, i % 2 == 0).0,
                list: (0..i).map(U256::from).collect(),
            })
            .collect();
        let sols: Vec<((U256, Address, bool), Vec<U256>)> = values
            .iter()
            .map(|v| ((v.inner.a, v.inner.b, v.inner.c), v.list.clone()))
            .collect();
        case_as!(
            &mut r,
            sol_data::Array<SolNested>,
            values,
            sols,
            &format!("nested struct[] len {len}")
        );
    }

    r.assert_clean("arrays of structs");
}

/// A static struct nested inside another struct: the inner struct's head width is what the outer
/// struct strides by, so a wrong width corrupts the outer layout too.
#[test]
fn structs_containing_narrow_structs_match_the_specification() {
    let mut r = Report::default();

    #[derive(Codec, Default, Debug, PartialEq, Clone)]
    struct Outer {
        head: U256,
        inner: NarrowStruct,
        tail: U256,
    }

    type SolOuter = (sol_data::Uint<256>, SolNarrow, sol_data::Uint<256>);

    for (label, value) in [
        (
            "outer{U256, narrow struct, U256} zeros",
            Outer {
                head: U256::ZERO,
                inner: NarrowStruct {
                    small: 0,
                    wide: 0,
                    signed: 0,
                },
                tail: U256::ZERO,
            },
        ),
        (
            "outer{U256, narrow struct, U256} populated",
            Outer {
                head: U256::from(7),
                inner: NarrowStruct {
                    small: 1,
                    wide: 2,
                    signed: -3,
                },
                tail: U256::from(9),
            },
        ),
    ] {
        let sol = (
            value.head,
            (value.inner.small, value.inner.wide, value.inner.signed),
            value.tail,
        );
        case_as!(&mut r, SolOuter, value, sol, label);
    }

    r.assert_clean("structs containing narrow structs");
}

// ---------------------------------------------------------------------------------------------
// Function arguments
//
// spec: "a call to the function f with parameters a_1, ..., a_n is encoded as
// function_selector(f) enc((a_1, ..., a_n))" - i.e. the arguments are one tuple. The selector
// itself is not this codec's concern; the tuple encoding is.
// ---------------------------------------------------------------------------------------------

/// `FunctionArgs` is not exported from the crate root, so this cannot be a generic helper with a
/// trait bound; each case is monomorphic instead.
macro_rules! function_args_case {
    ($report:expr, $sol:ty, $value:expr, $label:expr) => {{
        $report.checked += 1;
        let value = $value;
        let mut buf = BytesMut::new();
        fluentbase_codec::SolidityABI::encode_function_args(&value, &mut buf)
            .expect("encoding well-formed arguments must succeed");
        let ours = buf.to_vec();
        let theirs = <$sol as SolType>::abi_encode_params(&value);
        if ours != theirs {
            $report.fail(
                $label,
                "bytes",
                format!(
                    "ours  = {}\n      alloy = {}",
                    hex::encode(&ours),
                    hex::encode(&theirs)
                ),
            );
        }
    }};
}

#[test]
fn function_arguments_match_the_specification() {
    let mut r = Report::default();

    function_args_case!(
        &mut r,
        (sol_data::Uint<256>,),
        (U256::from(7),),
        "f(uint256)"
    );
    function_args_case!(
        &mut r,
        (sol_data::Uint<32>, sol_data::Uint<256>),
        (0u32, U256::ZERO),
        "f(uint32,uint256) all zero"
    );
    function_args_case!(
        &mut r,
        (sol_data::Uint<256>, sol_data::String),
        (U256::from(1), "x".to_string()),
        "f(uint256,string)"
    );
    function_args_case!(
        &mut r,
        (sol_data::Array<sol_data::Uint<256>>, sol_data::Address),
        (vec![U256::from(1), U256::from(2)], Address::ZERO),
        "f(uint256[],address)"
    );
    function_args_case!(
        &mut r,
        (sol_data::String, sol_data::Bytes),
        (String::new(), Bytes::new()),
        "f(string,bytes) both dynamic, both empty"
    );
    function_args_case!(
        &mut r,
        (sol_data::Array<sol_data::String>,),
        (vec!["a".to_string(), String::new(), "ccc".to_string()],),
        "f(string[]) with three elements"
    );

    r.assert_clean("function arguments");
}

// ---------------------------------------------------------------------------------------------
// Indexed event topics
//
// spec, "Encoding of Indexed Event Parameters":
//   - bytes and string are hashed over "just the string contents without any padding or length
//     prefix";
//   - a struct is "the concatenation of the encoding of its members, always padded to a multiple
//     of 32 bytes (even bytes and string)";
//   - an array, fixed or dynamic, is the concatenation of its elements, padded, "without any
//     length prefix";
//   - "a negative number is padded by sign extension"; "bytesNN types are padded on the right
//     while uintNN / intNN are padded on the left".
// ---------------------------------------------------------------------------------------------

macro_rules! topic_case {
    ($report:expr, $sol:ty, $value:expr, $label:expr) => {{
        $report.checked += 1;
        let value = $value;
        let ours = match fluentbase_codec::encode_indexed_topic(&value)
            .expect("encoding a well-formed topic must succeed")
        {
            fluentbase_codec::IndexedTopic::Word(word) => alloy_primitives::B256::new(word),
            fluentbase_codec::IndexedTopic::Preimage(preimage) => {
                alloy_primitives::keccak256(&preimage)
            }
        };
        let theirs = <$sol as alloy_sol_types::EventTopic>::encode_topic(&value).0;
        if ours != theirs {
            $report.fail($label, "topic", format!("ours = {ours}, alloy = {theirs}"));
        }
    }};
}

#[test]
fn indexed_topics_match_the_specification() {
    let mut r = Report::default();

    // Value types occupy the topic word directly.
    topic_case!(&mut r, sol_data::Bool, false, "bool false");
    topic_case!(&mut r, sol_data::Uint<8>, 0u8, "uint8 zero");
    topic_case!(&mut r, sol_data::Uint<64>, 0u64, "uint64 zero");
    topic_case!(&mut r, sol_data::Uint<64>, u64::MAX, "uint64 max");
    topic_case!(&mut r, sol_data::Int<32>, -1i32, "int32 -1, sign extended");
    topic_case!(&mut r, sol_data::Int<64>, i64::MIN, "int64 min");
    topic_case!(&mut r, sol_data::Uint<256>, U256::MAX, "uint256 max");
    topic_case!(&mut r, sol_data::Int<256>, I256::MINUS_ONE, "int256 -1");
    topic_case!(
        &mut r,
        sol_data::Address,
        Address::repeat_byte(0xab),
        "address"
    );
    topic_case!(
        &mut r,
        sol_data::FixedBytes<4>,
        FixedBytes::<4>::new([1, 2, 3, 4]),
        "bytes4, padded on the right"
    );

    // Reference types are hashed. bytes/string over raw contents, with no length and no padding.
    for len in [0usize, 1, 31, 32, 33] {
        topic_case!(
            &mut r,
            sol_data::Bytes,
            Bytes::from(vec![0xcd; len]),
            &format!("bytes of length {len}")
        );
        topic_case!(
            &mut r,
            sol_data::String,
            "y".repeat(len),
            &format!("string of length {len}")
        );
    }

    // Arrays: elements padded, no length prefix.
    for len in [0usize, 1, 2, 3] {
        topic_case!(
            &mut r,
            sol_data::Array<sol_data::Uint<256>>,
            (0..len).map(U256::from).collect::<Vec<_>>(),
            &format!("uint256[] len {len}")
        );
        topic_case!(
            &mut r,
            sol_data::Array<sol_data::Uint<32>>,
            (0..len as u32).collect::<Vec<_>>(),
            &format!("uint32[] len {len}, first element zero")
        );
        topic_case!(
            &mut r,
            sol_data::Array<sol_data::String>,
            (0..len).map(|i| "a".repeat(i)).collect::<Vec<_>>(),
            &format!("string[] len {len}, members padded to a word")
        );
        topic_case!(
            &mut r,
            sol_data::Array<sol_data::Bytes>,
            (0..len)
                .map(|i| Bytes::from(vec![1u8; i * 17]))
                .collect::<Vec<_>>(),
            &format!("bytes[] len {len}, members crossing word boundaries")
        );
    }

    // Fixed arrays are reference types too - hashed, not inlined.
    topic_case!(
        &mut r,
        sol_data::FixedArray<sol_data::Address, 2>,
        [Address::ZERO, Address::repeat_byte(2)],
        "address[2]"
    );
    topic_case!(
        &mut r,
        sol_data::FixedArray<sol_data::Uint<64>, 3>,
        [0u64, 1, u64::MAX],
        "uint64[3] containing zero"
    );

    // Tuples follow the struct rule: members concatenated in place, each padded to a word.
    topic_case!(
        &mut r,
        (sol_data::Uint<256>, sol_data::String),
        (U256::ZERO, "abc".to_string()),
        "(uint256,string)"
    );
    topic_case!(
        &mut r,
        (sol_data::Uint<32>, sol_data::Array<sol_data::Uint<256>>),
        (0u32, vec![U256::from(1), U256::from(2)]),
        "(uint32,uint256[]) with a zero member"
    );

    r.assert_clean("indexed topics");
}

// ---------------------------------------------------------------------------------------------
// Non-standard packed mode
//
// spec, "Non-standard Packed Mode":
//   - "types shorter than 32 bytes are concatenated directly, without padding or sign extension"
//   - "dynamic types are encoded in-place and without the length"
//   - "array elements are padded, but still encoded in-place"
//   - "structs as well as nested arrays are not supported"
//
// The worked example in the spec is int16(-1), bytes1(0x42), uint16(0x03), string("Hello, world!")
// encoding to 0xffff42000348656c6c6f2c20776f726c6421 - note int16(-1) is two bytes, not padded.
// ---------------------------------------------------------------------------------------------

macro_rules! packed_case {
    ($report:expr, $sol:ty, $value:expr, $label:expr) => {{
        $report.checked += 1;
        let value = $value;
        let mut buf = BytesMut::new();
        fluentbase_codec::SolidityPackedABI::encode(&value, &mut buf, 0)
            .expect("encoding a well-formed value must succeed");
        let ours = buf.to_vec();
        let theirs = <$sol as SolType>::abi_encode_packed(&value);
        if ours != theirs {
            $report.fail(
                $label,
                "packed bytes",
                format!(
                    "ours  = {} ({} bytes)\n      alloy = {} ({} bytes)",
                    hex::encode(&ours),
                    ours.len(),
                    hex::encode(&theirs),
                    theirs.len()
                ),
            );
        }
    }};
}

/// Same as `packed_case!`, but checks against bytes written out from the specification instead of
/// against alloy.
///
/// Needed where alloy is the one that departs from the spec: `stv_abi_encode_packed_to` for
/// `FixedArray` zero-pads every element on the left (`data_type.rs`, "Array elements are
/// left-padded to 32 bytes"), whereas the spec says an array element carries its own standard
/// encoding - which sign-extends a negative `int<M>` and pads `bytes<M>` on the *right*.
macro_rules! packed_spec_case {
    ($report:expr, $value:expr, $expected_hex:expr, $label:expr) => {{
        $report.checked += 1;
        let value = $value;
        let mut buf = BytesMut::new();
        fluentbase_codec::SolidityPackedABI::encode(&value, &mut buf, 0)
            .expect("encoding a well-formed value must succeed");
        let ours = hex::encode(buf.to_vec());
        if ours != $expected_hex {
            $report.fail(
                $label,
                "packed bytes",
                format!("ours  = {}\n      spec  = {}", ours, $expected_hex),
            );
        }
    }};
}

#[test]
fn packed_mode_matches_the_specification() {
    let mut r = Report::default();

    packed_case!(&mut r, sol_data::Bool, true, "bool true");
    packed_case!(&mut r, sol_data::Bool, false, "bool false");

    packed_case!(&mut r, sol_data::Uint<8>, 0x42u8, "uint8");
    packed_case!(
        &mut r,
        sol_data::Uint<16>,
        0x03u16,
        "uint16, from the spec example"
    );
    packed_case!(&mut r, sol_data::Uint<32>, 0u32, "uint32 zero");
    packed_case!(&mut r, sol_data::Uint<64>, u64::MAX, "uint64 max");
    packed_case!(&mut r, sol_data::Uint<256>, U256::from(1), "uint256");

    // "without padding or sign extension": int16(-1) is 0xffff, two bytes.
    packed_case!(
        &mut r,
        sol_data::Int<16>,
        -1i16,
        "int16 -1, no sign extension"
    );
    packed_case!(&mut r, sol_data::Int<32>, -1i32, "int32 -1");
    packed_case!(&mut r, sol_data::Int<32>, 0i32, "int32 zero");
    packed_case!(&mut r, sol_data::Int<64>, i64::MIN, "int64 min");

    packed_case!(
        &mut r,
        sol_data::Address,
        Address::repeat_byte(0xab),
        "address"
    );
    packed_case!(
        &mut r,
        sol_data::FixedBytes<1>,
        FixedBytes::<1>::new([0x42]),
        "bytes1, from the spec example"
    );
    packed_case!(
        &mut r,
        sol_data::FixedBytes<4>,
        FixedBytes::<4>::new([1, 2, 3, 4]),
        "bytes4"
    );
    packed_case!(
        &mut r,
        sol_data::FixedBytes<32>,
        FixedBytes::<32>::repeat_byte(7),
        "bytes32"
    );

    // "array elements are padded, but still encoded in-place"
    packed_case!(
        &mut r,
        sol_data::FixedArray<sol_data::Uint<16>, 3>,
        [1u16, 0, u16::MAX],
        "uint16[3], elements padded to a word"
    );
    packed_case!(
        &mut r,
        sol_data::FixedArray<sol_data::Address, 2>,
        [Address::ZERO, Address::repeat_byte(9)],
        "address[2]"
    );
    packed_case!(
        &mut r,
        sol_data::FixedArray<sol_data::Uint<256>, 2>,
        [U256::from(1), U256::from(2)],
        "uint256[2], already word-wide"
    );
    packed_case!(
        &mut r,
        sol_data::FixedArray<sol_data::Bool, 3>,
        [true, false, true],
        "bool[3]"
    );
    // The two cases below are checked against the spec rather than alloy - see
    // `packed_spec_case!`. An element's padding follows the element's own standard encoding:
    // "int<M>: ... padded on the higher-order (left) side with 0xff for negative X", and
    // "bytes<M>: enc(X) is the sequence of bytes in X padded with trailing zero-bytes".
    packed_spec_case!(
        &mut r,
        [-1i32, 0i32],
        concat!(
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "0000000000000000000000000000000000000000000000000000000000000000"
        ),
        "int32[2], the element's own padding sign-extends"
    );
    packed_spec_case!(
        &mut r,
        [
            FixedBytes::<4>::new([1, 2, 3, 4]),
            FixedBytes::<4>::new([5, 6, 7, 8])
        ],
        concat!(
            "0102030400000000000000000000000000000000000000000000000000000000",
            "0506070800000000000000000000000000000000000000000000000000000000"
        ),
        "bytes4[2], the element is padded on the right"
    );

    // An array that does not start on a word boundary: the scalar before it is unpadded, so the
    // array's elements are padded relative to where the array begins, not to absolute words.
    packed_case!(
        &mut r,
        (
            sol_data::Uint<8>,
            sol_data::FixedArray<sol_data::Uint<16>, 2>
        ),
        (0x42u8, [1u16, 2u16]),
        "uint8 then uint16[2], the array starts unaligned"
    );

    // Not covered, and not coverable: the spec allows `string`, `bytes` and dynamic arrays in
    // packed mode ("dynamic types are encoded in-place and without the length"), but
    // `SolidityPackedABI` is declared `static_only`, so `String` - which only implements
    // `Encoder<B, ALIGN, true, false>` - is rejected at compile time. That is a narrower API than
    // the spec, not wrong bytes; nothing here can assert on it.

    r.assert_clean("packed mode");
}

// ---------------------------------------------------------------------------------------------
// Shapes the tables above did not reach
//
// The groups so far cover each construct once. These are the combinations: a dynamic member
// between two static ones, a struct nested two levels deep, a fixed array inside a struct, an
// array of fixed arrays, and the integer widths that are neither a Rust primitive nor 256 bits.
// ---------------------------------------------------------------------------------------------

#[derive(Codec, Default, Debug, PartialEq, Clone)]
struct StructWithFixedArray {
    ids: [u32; 3],
    tail: U256,
}
type SolStructWithFixedArray = (
    sol_data::FixedArray<sol_data::Uint<32>, 3>,
    sol_data::Uint<256>,
);

/// Mirrors `LegacyInitialSettings` in `fluentbase-sdk`, which stores names as `[u8; 32]`.
#[derive(Codec, Default, Debug, PartialEq, Clone)]
struct StructWithByteArray {
    name: [u8; 32],
    decimals: u8,
}
type SolStructWithByteArray = (
    sol_data::FixedArray<sol_data::Uint<8>, 32>,
    sol_data::Uint<8>,
);

/// A dynamic member with static members on both sides: the head area has to keep a word for the
/// offset in the middle, and the tail has to land after both static members.
#[derive(Codec, Default, Debug, PartialEq, Clone)]
struct SandwichStruct {
    head: U256,
    inner: DynamicStruct,
    tail: U256,
}
type SolSandwich = (sol_data::Uint<256>, SolDynamic, sol_data::Uint<256>);

/// Two levels of nesting, both dynamic.
#[derive(Codec, Default, Debug, PartialEq, Clone)]
struct DeepStruct {
    id: U256,
    middle: NestedStruct,
    names: Vec<String>,
}
type SolDeep = (
    sol_data::Uint<256>,
    SolNested,
    sol_data::Array<sol_data::String>,
);

#[test]
fn unusual_integer_widths_match_the_specification() {
    let mut r = Report::default();

    // Widths that are not a Rust primitive: `Uint<BITS, LIMBS>` computes `BYTES` as
    // `ceil(BITS / 8)`, so the padding arithmetic differs from the `impl_int!` primitives.
    for v in [U24::ZERO, U24::from(1), U24::MAX] {
        case!(&mut r, sol_data::Uint<24>, v);
    }
    for v in [U40::ZERO, U40::from(0xff), U40::MAX] {
        case!(&mut r, sol_data::Uint<40>, v);
    }
    for v in [U160::ZERO, U160::from(7), U160::MAX] {
        case!(&mut r, sol_data::Uint<160>, v);
    }

    // uint128/int128 are `Uint`/`Signed` on our side and Rust primitives on alloy's.
    // The three widths whose `sol_to_rust` mapping lands on a Rust primitive: they delegate to
    // `Uint`/`Signed` rather than carrying their own padding arithmetic.
    for v in [0i8, -1, 1, i8::MIN, i8::MAX] {
        case!(&mut r, sol_data::Int<8>, v, &format!("int8 = {v}"));
    }
    for v in [0u128, 1, u128::MAX] {
        case!(
            &mut r,
            sol_data::Uint<128>,
            v,
            &format!("uint128 primitive = {v}")
        );
    }
    for v in [0i128, -1, i128::MIN, i128::MAX] {
        case!(
            &mut r,
            sol_data::Int<128>,
            v,
            &format!("int128 primitive = {v}")
        );
    }

    for v in [0u128, 1, u128::MAX] {
        case_as!(
            &mut r,
            sol_data::Uint<128>,
            U128::from(v),
            v,
            &format!("uint128 = {v}")
        );
    }
    for v in [0i128, -1, i128::MIN, i128::MAX] {
        case_as!(
            &mut r,
            sol_data::Int<128>,
            I128::try_from(v).unwrap(),
            v,
            &format!("int128 = {v}")
        );
    }

    r.assert_clean("unusual integer widths");
}

#[test]
fn arrays_of_signed_elements_match_the_specification() {
    let mut r = Report::default();

    // Every array group so far used unsigned elements, so nothing exercised sign extension
    // inside an element head.
    for len in [0usize, 1, 2, 3] {
        case!(
            &mut r,
            sol_data::Array<sol_data::Int<32>>,
            (0..len)
                .map(|i| if i % 2 == 0 { -(i as i32) - 1 } else { 0 })
                .collect::<Vec<_>>(),
            &format!("int32[] len {len} (alternating negative and zero)")
        );
        case!(
            &mut r,
            sol_data::Array<sol_data::Int<256>>,
            (0..len)
                .map(|i| if i % 2 == 0 {
                    I256::MINUS_ONE
                } else {
                    I256::ZERO
                })
                .collect::<Vec<_>>(),
            &format!("int256[] len {len}")
        );
    }

    r.assert_clean("arrays of signed elements");
}

#[test]
fn fixed_arrays_in_composites_match_the_specification() {
    let mut r = Report::default();

    // `T[k]` inside a struct, and `T[k]` as the element of a `T[]`. A fixed array is static, so
    // it belongs inline in the head area - its width is `k` element heads, not one offset word.
    let value = StructWithFixedArray {
        ids: [0, 1, u32::MAX],
        tail: U256::MAX,
    };
    let sol = (value.ids, value.tail);
    case_as!(
        &mut r,
        SolStructWithFixedArray,
        value,
        sol,
        "struct { uint32[3], uint256 }"
    );

    let mut name = [0u8; 32];
    name[..6].copy_from_slice(b"fluent");
    let value = StructWithByteArray { name, decimals: 18 };
    let sol = (value.name, value.decimals);
    case_as!(
        &mut r,
        SolStructWithByteArray,
        value,
        sol,
        "struct { uint8[32], uint8 }"
    );

    for len in [0usize, 1, 2, 3] {
        case!(
            &mut r,
            sol_data::Array<sol_data::FixedArray<sol_data::Uint<32>, 3>>,
            (0..len)
                .map(|i| [i as u32, 0, u32::MAX])
                .collect::<Vec<_>>(),
            &format!("uint32[3][] len {len}")
        );
    }

    // A fixed array whose elements are static structs: `k` inline struct encodings, no offsets.
    let a = StaticStruct {
        a: U256::ZERO,
        b: Address::ZERO,
        c: false,
    };
    let b = StaticStruct {
        a: U256::MAX,
        b: Address::repeat_byte(9),
        c: true,
    };
    let value = [a, b];
    let sol = [(a.a, a.b, a.c), (b.a, b.b, b.c)];
    case_as!(
        &mut r,
        sol_data::FixedArray<SolStatic, 2>,
        value,
        sol,
        "struct[2], inline"
    );

    r.assert_clean("fixed arrays in composites");
}

#[test]
fn fixed_arrays_of_dynamic_elements_match_the_specification() {
    let mut r = Report::default();

    // `T[k]` is dynamic exactly when `T` is - `string[3]` is a dynamic type whose head is one
    // offset word and whose body is three offset words followed by the three tails. Until the
    // `Copy` bound came off `[T; N]` these types could not be constructed at all, which is a whole
    // branch of the specification the codec could not reach.
    case!(
        &mut r,
        sol_data::FixedArray<sol_data::String, 3>,
        ["a".to_string(), String::new(), "x".repeat(40)],
        "string[3], including an empty and an over-a-word element"
    );
    case!(
        &mut r,
        sol_data::FixedArray<sol_data::Bytes, 2>,
        [Bytes::from_static(&[1u8; 5]), Bytes::new()],
        "bytes[2]"
    );
    case!(
        &mut r,
        sol_data::FixedArray<sol_data::Bytes, 1>,
        [Bytes::from_static(&[7u8; 33])],
        "bytes[1] crossing a word boundary"
    );

    // A fixed array of dynamic tuples: two levels of offsets, the inner ones relative to their own
    // tuple and the outer ones relative to the array's body.
    case!(
        &mut r,
        sol_data::FixedArray<(sol_data::Bytes, sol_data::Bytes), 2>,
        [
            (Bytes::from_static(&[1u8; 3]), Bytes::from_static(&[2u8; 40])),
            (Bytes::new(), Bytes::from_static(&[9u8; 33]))
        ],
        "(bytes,bytes)[2]"
    );

    // A fixed array whose elements are themselves dynamic arrays, and one nested two deep.
    case!(
        &mut r,
        sol_data::FixedArray<sol_data::Array<sol_data::Uint<256>>, 2>,
        [vec![U256::ZERO, U256::MAX], Vec::new()],
        "uint256[][2], second element empty"
    );
    case!(
        &mut r,
        sol_data::FixedArray<sol_data::FixedArray<sol_data::Bytes, 3>, 2>,
        [
            [
                Bytes::from_static(&[1u8; 1]),
                Bytes::from_static(&[2u8; 2]),
                Bytes::from_static(&[3u8; 40])
            ],
            [Bytes::new(), Bytes::from_static(&[5u8; 5]), Bytes::new()]
        ],
        "bytes[3][2]"
    );

    // A dynamic fixed array as a member, so its offset word sits in a head area with other members
    // on both sides.
    case!(
        &mut r,
        (
            sol_data::Uint<256>,
            sol_data::FixedArray<sol_data::String, 2>,
            sol_data::Address
        ),
        (
            U256::MAX,
            ["first".to_string(), "second".to_string()],
            Address::repeat_byte(0xcd)
        ),
        "(uint256, string[2], address)"
    );

    // A dynamic array of dynamic fixed arrays - the length prefix and the fixed head both present.
    for len in [0usize, 1, 2, 3] {
        case!(
            &mut r,
            sol_data::Array<sol_data::FixedArray<sol_data::String, 2>>,
            (0..len)
                .map(|i| ["a".repeat(i), "b".repeat(i * 33)])
                .collect::<Vec<_>>(),
            &format!("string[2][] len {len}")
        );
    }

    r.assert_clean("fixed arrays of dynamic elements");
}

#[test]
fn deeply_nested_composites_match_the_specification() {
    let mut r = Report::default();

    // A dynamic member between two static ones.
    for (label, name, blob) in [
        ("sandwich, empty dynamic member", "", &[][..]),
        ("sandwich, populated", "fluent", &[1u8, 2, 3][..]),
        (
            "sandwich, member over one word",
            "0123456789012345678901234567890123",
            &[9u8; 40][..],
        ),
    ] {
        let (inner, inner_sol) = dynamic_struct(7, name, blob);
        let value = SandwichStruct {
            head: U256::from(1),
            inner,
            tail: U256::MAX,
        };
        let sol = (value.head, inner_sol, value.tail);
        case_as!(&mut r, SolSandwich, value, sol, label);
    }

    // Two levels of dynamic nesting, swept over the inner lengths.
    for len in [0usize, 1, 2, 3] {
        let inner = StaticStruct {
            a: U256::from(len),
            b: Address::repeat_byte(len as u8),
            c: len % 2 == 0,
        };
        let value = DeepStruct {
            id: U256::from(len),
            middle: NestedStruct {
                inner,
                list: (0..len).map(U256::from).collect(),
            },
            names: (0..len).map(|i| "x".repeat(i * 20)).collect(),
        };
        let sol = (
            value.id,
            ((inner.a, inner.b, inner.c), value.middle.list.clone()),
            value.names.clone(),
        );
        case_as!(
            &mut r,
            SolDeep,
            value,
            sol,
            &format!("struct in struct, inner len {len}")
        );
    }

    // An array of arrays whose innermost elements are dynamic, and an array of arrays of structs.
    for len in [0usize, 1, 2, 3] {
        case!(
            &mut r,
            sol_data::Array<sol_data::Array<sol_data::String>>,
            (0..len)
                .map(|i| (0..i).map(|j| "y".repeat(j * 33)).collect::<Vec<_>>())
                .collect::<Vec<_>>(),
            &format!("string[][] len {len}")
        );

        let values: Vec<Vec<NarrowStruct>> = (0..len)
            .map(|i| {
                (0..i)
                    .map(|j| NarrowStruct {
                        small: j as u32,
                        wide: 0,
                        signed: -(j as i32),
                    })
                    .collect()
            })
            .collect();
        let sol: Vec<Vec<(u32, u64, i32)>> = values
            .iter()
            .map(|row| row.iter().map(|v| (v.small, v.wide, v.signed)).collect())
            .collect();
        case_as!(
            &mut r,
            sol_data::Array<sol_data::Array<SolNarrow>>,
            values,
            sol,
            &format!("narrow struct[][] len {len}")
        );
    }

    // A tuple used directly as a top-level value, rather than through `encode_function_args`.
    case!(
        &mut r,
        (sol_data::Uint<256>, sol_data::String),
        (U256::MAX, "top level tuple".to_string()),
        "(uint256, string) as a value"
    );

    r.assert_clean("deeply nested composites");
}

// ---------------------------------------------------------------------------------------------
// The specification's own worked examples
//
// These are the only vectors in the suite that owe nothing to alloy: the bytes are copied from the
// "Examples" and "Use of Dynamic Types" sections of the spec, which spell out every word and the
// reasoning behind each offset. If our encoder and alloy ever agreed on something wrong, this is
// the group that would still say so.
//
// The four-byte selector is stripped: `encode_function_args` encodes the argument tuple, and the
// selector is not this codec's concern.
// ---------------------------------------------------------------------------------------------

macro_rules! spec_vector_case {
    ($report:expr, $value:expr, $expected:expr, $label:expr) => {{
        $report.checked += 1;
        let value = $value;
        let mut buf = BytesMut::new();
        fluentbase_codec::SolidityABI::encode_function_args(&value, &mut buf)
            .expect("encoding well-formed arguments must succeed");
        let ours = hex::encode(buf.to_vec());
        let expected: String = $expected.split_whitespace().collect();
        if ours != expected {
            $report.fail(
                $label,
                "bytes",
                format!("ours  = {}\n      spec  = {}", ours, expected),
            );
        }
    }};
}

/// One-element tuples, at the top level and nested.
///
/// The specification gives one-element tuples no special treatment: `(string)` is a tuple like any
/// other, so its member's offset is measured from the start of the tuple's own encoding. A
/// one-element tuple that is written at the very start of a buffer masks a mistake in that origin,
/// because "start of the tuple" and "start of the buffer" then coincide; nesting one after another
/// member separates the two. Every case here is therefore paired: the same shape alone and the
/// same shape with something in front of it.
#[test]
fn one_element_tuples_match_the_specification() {
    let mut r = Report::default();

    // The whole defect in five words, written out so it can be read rather than run.
    //
    //   word 0   0x40   offset to the tail of member 0
    //   word 1   0xff   member 1, the uint256
    //   word 2   0x20   member 0 is a tuple, and this is its own head: the offset to its string
    //   word 3   0x02   the string's length
    //   word 4   "hi"
    //
    // Word 2 is the one that used to be missing. Everything after it shifted up by a word, so the
    // offset in word 0 pointed at the length instead of at the tuple's head, and a decoder
    // following it jumped to byte 2.
    spec_vector_case!(
        &mut r,
        (("hi".to_string(),), U256::from(255)),
        "0000000000000000000000000000000000000000000000000000000000000040
         00000000000000000000000000000000000000000000000000000000000000ff
         0000000000000000000000000000000000000000000000000000000000000020
         0000000000000000000000000000000000000000000000000000000000000002
         6869000000000000000000000000000000000000000000000000000000000000",
        "((string),uint256) with ((hi), 255)"
    );

    for text in ["", "fluent", "0123456789012345678901234567890123"] {
        let label = |what: &str| format!("{what}, member {:?}", text);

        // Alone - the arrangement that coincides with a correct one.
        case!(
            &mut r,
            (sol_data::String,),
            (text.to_string(),),
            &label("(string)")
        );

        // After a static member, so the tuple no longer starts at offset zero.
        case!(
            &mut r,
            (sol_data::Uint<256>, (sol_data::String,)),
            (U256::from(1), (text.to_string(),)),
            &label("(uint256,(string))")
        );

        // Before a static member, so the tuple's tail is not the end of the buffer either.
        case!(
            &mut r,
            ((sol_data::String,), sol_data::Uint<256>),
            ((text.to_string(),), U256::MAX),
            &label("((string),uint256)")
        );

        // Two levels, so the inner tuple's origin is itself not the buffer's start.
        case!(
            &mut r,
            (sol_data::Uint<256>, ((sol_data::String,),)),
            (U256::from(2), ((text.to_string(),),)),
            &label("(uint256,((string)))")
        );

        // Between two dynamic members, which puts an entire tail between head and body.
        case!(
            &mut r,
            (sol_data::String, (sol_data::String,), sol_data::String),
            ("head".to_string(), (text.to_string(),), "tail".to_string()),
            &label("(string,(string),string)")
        );
    }

    // A one-element tuple that is static keeps its member inline, before and after a sibling.
    case!(
        &mut r,
        (sol_data::Uint<256>,),
        (U256::from(7),),
        "(uint256), alone"
    );
    case!(
        &mut r,
        (sol_data::Address, (sol_data::Uint<256>,)),
        (Address::repeat_byte(3), (U256::from(7),)),
        "(address,(uint256))"
    );

    // An array of one-element tuples: every element's offset is relative to the array body.
    for len in [0usize, 1, 2, 3] {
        case!(
            &mut r,
            sol_data::Array<(sol_data::String,)>,
            (0..len).map(|i| ("z".repeat(i * 33),)).collect::<Vec<_>>(),
            &format!("(string)[] len {len}")
        );
    }

    r.assert_clean("one-element tuples");
}

#[test]
fn the_specifications_worked_examples_match_byte_for_byte() {
    let mut r = Report::default();

    // bar(bytes3[2]) with ["abc", "def"] - a static fixed array, left-aligned elements.
    spec_vector_case!(
        &mut r,
        ([FixedBytes::<3>::new(*b"abc"), FixedBytes::<3>::new(*b"def")],),
        "6162630000000000000000000000000000000000000000000000000000000000
         6465660000000000000000000000000000000000000000000000000000000000",
        "bar(bytes3[2]) with [abc, def]"
    );

    // baz(uint32,bool) with (69, true).
    spec_vector_case!(
        &mut r,
        (69u32, true),
        "0000000000000000000000000000000000000000000000000000000000000045
         0000000000000000000000000000000000000000000000000000000000000001",
        "baz(uint32,bool) with (69, true)"
    );

    // sam(bytes,bool,uint256[]) with ("dave", true, [1,2,3]) - two dynamic arguments with a
    // static one between them, so both offsets have to skip the whole head.
    spec_vector_case!(
        &mut r,
        (
            Bytes::copy_from_slice(b"dave"),
            true,
            vec![U256::from(1), U256::from(2), U256::from(3)]
        ),
        "0000000000000000000000000000000000000000000000000000000000000060
         0000000000000000000000000000000000000000000000000000000000000001
         00000000000000000000000000000000000000000000000000000000000000a0
         0000000000000000000000000000000000000000000000000000000000000004
         6461766500000000000000000000000000000000000000000000000000000000
         0000000000000000000000000000000000000000000000000000000000000003
         0000000000000000000000000000000000000000000000000000000000000001
         0000000000000000000000000000000000000000000000000000000000000002
         0000000000000000000000000000000000000000000000000000000000000003",
        "sam(bytes,bool,uint256[]) with (dave, true, [1,2,3])"
    );

    // f(uint256,uint32[],bytes10,bytes) - the spec's "Use of Dynamic Types" example. The second
    // offset (0xe0) is the first offset plus the size of the first data part, which is the one
    // piece of arithmetic a codec gets wrong quietly.
    spec_vector_case!(
        &mut r,
        (
            U256::from(0x123),
            vec![0x456u32, 0x789],
            FixedBytes::<10>::new(*b"1234567890"),
            Bytes::copy_from_slice(b"Hello, world!")
        ),
        "0000000000000000000000000000000000000000000000000000000000000123
         0000000000000000000000000000000000000000000000000000000000000080
         3132333435363738393000000000000000000000000000000000000000000000
         00000000000000000000000000000000000000000000000000000000000000e0
         0000000000000000000000000000000000000000000000000000000000000002
         0000000000000000000000000000000000000000000000000000000000000456
         0000000000000000000000000000000000000000000000000000000000000789
         000000000000000000000000000000000000000000000000000000000000000d
         48656c6c6f2c20776f726c642100000000000000000000000000000000000000",
        "f(uint256,uint32[],bytes10,bytes)"
    );

    // g(uint256[][],string[]) - nested dynamic arrays, where every inner offset is relative to the
    // start of its own array rather than to the buffer. Offset words are written as absolute
    // buffer positions, so this is the case that would break if a non-zero starting offset ever
    // became reachable.
    spec_vector_case!(
        &mut r,
        (
            vec![vec![U256::from(1), U256::from(2)], vec![U256::from(3)]],
            vec!["one".to_string(), "two".to_string(), "three".to_string()]
        ),
        "0000000000000000000000000000000000000000000000000000000000000040
         0000000000000000000000000000000000000000000000000000000000000140
         0000000000000000000000000000000000000000000000000000000000000002
         0000000000000000000000000000000000000000000000000000000000000040
         00000000000000000000000000000000000000000000000000000000000000a0
         0000000000000000000000000000000000000000000000000000000000000002
         0000000000000000000000000000000000000000000000000000000000000001
         0000000000000000000000000000000000000000000000000000000000000002
         0000000000000000000000000000000000000000000000000000000000000001
         0000000000000000000000000000000000000000000000000000000000000003
         0000000000000000000000000000000000000000000000000000000000000003
         0000000000000000000000000000000000000000000000000000000000000060
         00000000000000000000000000000000000000000000000000000000000000a0
         00000000000000000000000000000000000000000000000000000000000000e0
         0000000000000000000000000000000000000000000000000000000000000003
         6f6e650000000000000000000000000000000000000000000000000000000000
         0000000000000000000000000000000000000000000000000000000000000003
         74776f0000000000000000000000000000000000000000000000000000000000
         0000000000000000000000000000000000000000000000000000000000000005
         7468726565000000000000000000000000000000000000000000000000000000",
        "g(uint256[][],string[])"
    );

    r.assert_clean("specification worked examples");
}

// ---------------------------------------------------------------------------------------------
// Invariants that hold for every shape
//
// The tables check values. These check the metadata the codec exports about a type, which callers
// rely on and which no byte comparison touches: whether the type is dynamic, and whether the
// header arithmetic agrees with what `encode` actually wrote.
// ---------------------------------------------------------------------------------------------

/// `IS_DYNAMIC` against alloy's `DYNAMIC`, and the spec's rule that a standard encoding is always
/// a whole number of words.
macro_rules! invariant_case {
    ($report:expr, $rust:ty, $sol:ty, $value:expr, $label:expr) => {{
        $report.checked += 2;
        let value: $rust = $value;

        let ours = <$rust as Encoder<BigEndian, 32, true, false>>::IS_DYNAMIC;
        let theirs = <$sol as SolType>::DYNAMIC;
        if ours != theirs {
            $report.fail(
                $label,
                "IS_DYNAMIC",
                format!("ours = {ours}, alloy = {theirs}"),
            );
        }

        let encoded = our_encoding(&value);
        if !encoded.len().is_multiple_of(32) {
            $report.fail(
                $label,
                "word multiple",
                format!("encoding is {} bytes, not a multiple of 32", encoded.len()),
            );
        }
    }};
}

#[test]
fn type_metadata_matches_the_specification() {
    let mut r = Report::default();

    invariant_case!(&mut r, bool, sol_data::Bool, true, "bool");
    invariant_case!(&mut r, u32, sol_data::Uint<32>, 7, "uint32");
    invariant_case!(&mut r, U256, sol_data::Uint<256>, U256::MAX, "uint256");
    invariant_case!(
        &mut r,
        Address,
        sol_data::Address,
        Address::repeat_byte(3),
        "address"
    );
    invariant_case!(
        &mut r,
        FixedBytes<4>,
        sol_data::FixedBytes<4>,
        FixedBytes::<4>::repeat_byte(1),
        "bytes4"
    );
    invariant_case!(
        &mut r,
        String,
        sol_data::String,
        "abc".to_string(),
        "string"
    );
    invariant_case!(
        &mut r,
        Bytes,
        sol_data::Bytes,
        Bytes::from_static(b"abc"),
        "bytes"
    );
    invariant_case!(
        &mut r,
        Vec<U256>,
        sol_data::Array<sol_data::Uint<256>>,
        vec![U256::ZERO, U256::MAX],
        "uint256[]"
    );
    invariant_case!(
        &mut r,
        Vec<String>,
        sol_data::Array<sol_data::String>,
        vec!["a".to_string()],
        "string[]"
    );
    invariant_case!(
        &mut r,
        Vec<Vec<U256>>,
        sol_data::Array<sol_data::Array<sol_data::Uint<256>>>,
        vec![vec![U256::ONE]],
        "uint256[][]"
    );
    invariant_case!(
        &mut r,
        [u32; 3],
        sol_data::FixedArray<sol_data::Uint<32>, 3>,
        [1, 2, 3],
        "uint32[3]"
    );
    invariant_case!(
        &mut r,
        StaticStruct,
        SolStatic,
        StaticStruct {
            a: U256::ONE,
            b: Address::ZERO,
            c: true
        },
        "static struct"
    );
    invariant_case!(
        &mut r,
        NarrowStruct,
        SolNarrow,
        NarrowStruct {
            small: 1,
            wide: 2,
            signed: -3
        },
        "narrow struct"
    );
    invariant_case!(
        &mut r,
        DynamicStruct,
        SolDynamic,
        dynamic_struct(1, "a", &[2u8]).0,
        "dynamic struct"
    );
    invariant_case!(
        &mut r,
        NestedStruct,
        SolNested,
        NestedStruct {
            inner: StaticStruct {
                a: U256::ONE,
                b: Address::ZERO,
                c: false
            },
            list: vec![U256::ONE],
        },
        "nested struct"
    );
    invariant_case!(
        &mut r,
        (U256, String),
        (sol_data::Uint<256>, sol_data::String),
        (U256::ONE, "a".to_string()),
        "(uint256, string)"
    );

    r.assert_clean("type metadata");
}

/// `partial_decode` and `size_hint` against what `encode` actually produced.
///
/// Both are public and neither is exercised anywhere else in this suite - `partial_decode` in
/// particular carries its own width arithmetic, which is exactly the arithmetic the packed-mode
/// fix had to change.
macro_rules! header_arithmetic_case {
    ($report:expr, $rust:ty, $value:expr, $label:expr) => {{
        $report.checked += 3;
        let value: $rust = $value;
        let encoded = our_encoding(&value);
        let buf = bytes::Bytes::from(encoded.clone());

        let is_dynamic = <$rust as Encoder<BigEndian, 32, true, false>>::IS_DYNAMIC;

        match <$rust as Encoder<BigEndian, 32, true, false>>::partial_decode(&buf, 0) {
            Ok((offset, size)) => {
                // Whatever the region means for this type, it has to be inside the encoding.
                if offset + size > encoded.len() {
                    $report.fail(
                        $label,
                        "partial_decode",
                        format!(
                            "claims bytes {}..{} of a {}-byte encoding",
                            offset,
                            offset + size,
                            encoded.len()
                        ),
                    );
                }
                // For a static type there is no head/tail split, so the region is the whole thing.
                if !is_dynamic && (offset, size) != (0, encoded.len()) {
                    $report.fail(
                        $label,
                        "partial_decode",
                        format!(
                            "static type reported ({offset}, {size}), encoding is {} bytes",
                            encoded.len()
                        ),
                    );
                }
            }
            Err(error) => $report.fail($label, "partial_decode", format!("rejected: {error:?}")),
        }

        // `size_hint` is documented as the number of bytes needed to encode the value; for a
        // static type that is the encoding itself. Dynamic types report their head only, which is
        // a different contract and is not asserted here.
        if !is_dynamic {
            let hint = <$rust as Encoder<BigEndian, 32, true, false>>::size_hint(&value);
            if hint != encoded.len() {
                $report.fail(
                    $label,
                    "size_hint",
                    format!("hint {hint}, encoding {} bytes", encoded.len()),
                );
            }
        }
    }};
}

#[test]
fn header_arithmetic_agrees_with_the_encoding() {
    let mut r = Report::default();

    header_arithmetic_case!(&mut r, bool, true, "bool");
    header_arithmetic_case!(&mut r, u32, u32::MAX, "uint32");
    header_arithmetic_case!(&mut r, u64, 0, "uint64 zero");
    header_arithmetic_case!(&mut r, i32, -1, "int32 -1");
    header_arithmetic_case!(&mut r, U256, U256::MAX, "uint256");
    header_arithmetic_case!(&mut r, I256, I256::MINUS_ONE, "int256 -1");
    header_arithmetic_case!(&mut r, Address, Address::repeat_byte(0xab), "address");
    header_arithmetic_case!(
        &mut r,
        FixedBytes<4>,
        FixedBytes::<4>::repeat_byte(1),
        "bytes4"
    );
    header_arithmetic_case!(
        &mut r,
        FixedBytes<32>,
        FixedBytes::<32>::repeat_byte(2),
        "bytes32"
    );
    header_arithmetic_case!(&mut r, [u32; 3], [1, 2, 3], "uint32[3]");
    header_arithmetic_case!(
        &mut r,
        StaticStruct,
        StaticStruct {
            a: U256::MAX,
            b: Address::repeat_byte(9),
            c: true
        },
        "static struct"
    );
    header_arithmetic_case!(
        &mut r,
        NarrowStruct,
        NarrowStruct {
            small: 1,
            wide: 2,
            signed: -3
        },
        "narrow struct"
    );

    // Dynamic types: only the containment check applies.
    header_arithmetic_case!(&mut r, String, "hello".to_string(), "string");
    header_arithmetic_case!(&mut r, Bytes, Bytes::from_static(&[1u8; 40]), "bytes");
    header_arithmetic_case!(&mut r, Vec<U256>, vec![U256::ONE, U256::MAX], "uint256[]");
    header_arithmetic_case!(
        &mut r,
        Vec<String>,
        vec!["a".to_string(), "bb".to_string()],
        "string[]"
    );

    r.assert_clean("header arithmetic");
}

/// `Option<T>` is a static type, so `Some` and `None` must occupy - and report - the same width.
///
/// The two branches of its `partial_decode` compute the inner width differently: `Some` asks
/// `T::partial_decode`, `None` uses `align_up::<ALIGN>(T::HEADER_SIZE)`. They agree only when
/// `T::partial_decode` reports the aligned width.
#[test]
fn optional_members_report_one_width() {
    let mut r = Report::default();

    macro_rules! option_case {
        ($typ:ty, $value:expr, $label:expr) => {{
            r.checked += 1;
            let some: Option<$typ> = Some($value);
            let none: Option<$typ> = None;

            let mut some_buf = BytesMut::new();
            <Option<$typ> as Encoder<BigEndian, 32, true, false>>::encode(&some, &mut some_buf, 0)
                .unwrap();
            let mut none_buf = BytesMut::new();
            <Option<$typ> as Encoder<BigEndian, 32, true, false>>::encode(&none, &mut none_buf, 0)
                .unwrap();

            let some_bytes = bytes::Bytes::from(some_buf.to_vec());
            let none_bytes = bytes::Bytes::from(none_buf.to_vec());
            let some_width = <Option<$typ> as Encoder<BigEndian, 32, true, false>>::partial_decode(
                &some_bytes,
                0,
            )
            .unwrap()
            .1;
            let none_width = <Option<$typ> as Encoder<BigEndian, 32, true, false>>::partial_decode(
                &none_bytes,
                0,
            )
            .unwrap()
            .1;

            if some_width != none_width {
                r.fail(
                    $label,
                    "partial_decode",
                    format!("Some reports {some_width}, None reports {none_width}"),
                );
            }
        }};
    }

    option_case!(u32, 7u32, "Option<uint32>");
    option_case!(u64, 7u64, "Option<uint64>");
    option_case!(bool, true, "Option<bool>");
    option_case!(U256, U256::MAX, "Option<uint256>");
    option_case!(Address, Address::repeat_byte(1), "Option<address>");

    r.assert_clean("optional members");
}

// ---------------------------------------------------------------------------------------------
// Randomised layer
//
// The tables above pin the shapes and lengths that the specification and the known defects call
// out. This layer sweeps the same shapes across generated lengths and values, which is what
// catches a stride that is right at one length and wrong at the next - the exact form the array
// defect took.
//
// Adding a shape is one `prop_case!` line: a name, the alloy type, and a strategy. The four
// directions come from the same `check` the tables use, so a randomised failure reads identically
// to a table failure.
// ---------------------------------------------------------------------------------------------

mod randomised {
    use super::*;
    use proptest::prelude::*;

    /// Container lengths worth sweeping. The array defect first showed up at 2 and 3 elements;
    /// nothing in the
    /// encoding changes shape past a handful, and every case costs four encode/decode rounds.
    const LENGTHS: core::ops::RangeInclusive<usize> = 0..=6;

    // Value strategies. Each mixes the boundary values into the uniform distribution, because
    // uniform sampling essentially never produces 0 or MAX and both have their own padding rules.

    fn u32_value() -> impl Strategy<Value = u32> {
        prop_oneof![Just(0u32), Just(u32::MAX), any::<u32>()]
    }

    fn u64_value() -> impl Strategy<Value = u64> {
        prop_oneof![Just(0u64), Just(u64::MAX), any::<u64>()]
    }

    fn i32_value() -> impl Strategy<Value = i32> {
        prop_oneof![
            Just(0i32),
            Just(-1i32),
            Just(i32::MIN),
            Just(i32::MAX),
            any::<i32>()
        ]
    }

    fn u256_value() -> impl Strategy<Value = U256> {
        prop_oneof![
            Just(U256::ZERO),
            Just(U256::MAX),
            any::<[u8; 32]>().prop_map(U256::from_be_bytes),
        ]
    }

    fn address_value() -> impl Strategy<Value = Address> {
        any::<[u8; 20]>().prop_map(Address::from)
    }

    /// Lengths around the word boundary matter for `string`/`bytes`: the tail is padded to a
    /// multiple of 32, so 31, 32 and 33 characters take different numbers of words.
    fn text_value() -> impl Strategy<Value = String> {
        prop_oneof![Just(String::new()), "\\PC{0,40}", "\\PC{31,33}"]
    }

    fn bytes_value() -> impl Strategy<Value = Bytes> {
        proptest::collection::vec(any::<u8>(), 0..40).prop_map(Bytes::from)
    }

    fn static_struct_value() -> impl Strategy<Value = StaticStruct> {
        (u256_value(), address_value(), any::<bool>()).prop_map(|(a, b, c)| StaticStruct {
            a,
            b,
            c,
        })
    }

    fn narrow_struct_value() -> impl Strategy<Value = NarrowStruct> {
        (u32_value(), u64_value(), i32_value()).prop_map(|(small, wide, signed)| NarrowStruct {
            small,
            wide,
            signed,
        })
    }

    fn dynamic_struct_value() -> impl Strategy<Value = DynamicStruct> {
        (u256_value(), text_value(), bytes_value()).prop_map(|(id, name, blob)| DynamicStruct {
            id,
            name,
            blob,
        })
    }

    /// One randomised shape.
    ///
    /// The three-argument form is for values whose Rust type is also alloy's `RustType`; the
    /// four-argument form takes a closure mapping our value onto alloy's, for derive structs.
    macro_rules! prop_case {
        ($name:ident, $sol:ty, $strategy:expr) => {
            prop_case!($name, $sol, $strategy, Clone::clone);
        };
        ($name:ident, $sol:ty, $strategy:expr, $to_sol:expr) => {
            proptest! {
                #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]
                #[test]
                fn $name(value in $strategy) {
                    let sol = ($to_sol)(&value);
                    let mut report = Report::default();
                    check::<_, $sol>(&mut report, stringify!($name), &value, &sol);
                    report.assert_clean(stringify!($name));
                }
            }
        };
    }

    prop_case!(
        scalars,
        SolStatic,
        (u256_value(), address_value(), any::<bool>())
    );

    prop_case!(
        arrays_of_words,
        sol_data::Array<sol_data::Uint<256>>,
        proptest::collection::vec(u256_value(), LENGTHS)
    );

    prop_case!(
        arrays_of_text,
        sol_data::Array<sol_data::String>,
        proptest::collection::vec(text_value(), LENGTHS)
    );

    prop_case!(
        arrays_of_bytes,
        sol_data::Array<sol_data::Bytes>,
        proptest::collection::vec(bytes_value(), LENGTHS)
    );

    prop_case!(
        nested_arrays,
        sol_data::Array<sol_data::Array<sol_data::Uint<256>>>,
        proptest::collection::vec(proptest::collection::vec(u256_value(), LENGTHS), LENGTHS)
    );

    prop_case!(
        arrays_of_dynamic_tuples,
        sol_data::Array<(sol_data::Uint<256>, sol_data::String)>,
        proptest::collection::vec((u256_value(), text_value()), LENGTHS)
    );

    // The three shapes below are the ones the array defect broke: `Vec` strode by a `HEADER_SIZE`
    // that was
    // neither the head width of a dynamic member nor the aligned inline width of a static struct.
    prop_case!(
        arrays_of_static_structs,
        sol_data::Array<SolStatic>,
        proptest::collection::vec(static_struct_value(), LENGTHS),
        |values: &Vec<StaticStruct>| values.iter().map(|v| (v.a, v.b, v.c)).collect::<Vec<_>>()
    );

    prop_case!(
        arrays_of_narrow_structs,
        sol_data::Array<SolNarrow>,
        proptest::collection::vec(narrow_struct_value(), LENGTHS),
        |values: &Vec<NarrowStruct>| values
            .iter()
            .map(|v| (v.small, v.wide, v.signed))
            .collect::<Vec<_>>()
    );

    prop_case!(
        arrays_of_dynamic_structs,
        sol_data::Array<SolDynamic>,
        proptest::collection::vec(dynamic_struct_value(), LENGTHS),
        |values: &Vec<DynamicStruct>| values
            .iter()
            .map(|v| (v.id, v.name.clone(), v.blob.clone()))
            .collect::<Vec<_>>()
    );

    prop_case!(
        structs_with_a_nested_array,
        SolNested,
        (
            static_struct_value(),
            proptest::collection::vec(u256_value(), LENGTHS)
        )
            .prop_map(|(inner, list)| NestedStruct { inner, list }),
        |v: &NestedStruct| ((v.inner.a, v.inner.b, v.inner.c), v.list.clone())
    );

    fn nested_struct_value() -> impl Strategy<Value = NestedStruct> {
        (
            static_struct_value(),
            proptest::collection::vec(u256_value(), LENGTHS),
        )
            .prop_map(|(inner, list)| NestedStruct { inner, list })
    }

    fn fixed_triple_value() -> impl Strategy<Value = [u32; 3]> {
        (u32_value(), u32_value(), u32_value()).prop_map(|(a, b, c)| [a, b, c])
    }

    // The shapes below mirror the four table groups added on the second pass, which the layer
    // above did not reach.

    prop_case!(
        arrays_of_signed,
        sol_data::Array<sol_data::Int<32>>,
        proptest::collection::vec(i32_value(), LENGTHS)
    );

    prop_case!(
        wide_unsigned,
        sol_data::Uint<160>,
        any::<[u8; 20]>().prop_map(|bytes| U160::from_be_bytes(bytes))
    );

    prop_case!(
        narrow_unsigned,
        sol_data::Uint<24>,
        any::<u32>().prop_map(|v| U24::from(v & 0x00ff_ffff))
    );

    prop_case!(
        unsigned_128,
        sol_data::Uint<128>,
        any::<u128>().prop_map(U128::from),
        |v: &U128| v.to::<u128>()
    );

    prop_case!(
        arrays_of_fixed_arrays,
        sol_data::Array<sol_data::FixedArray<sol_data::Uint<32>, 3>>,
        proptest::collection::vec(fixed_triple_value(), LENGTHS)
    );

    prop_case!(
        structs_with_a_fixed_array,
        SolStructWithFixedArray,
        (fixed_triple_value(), u256_value())
            .prop_map(|(ids, tail)| StructWithFixedArray { ids, tail }),
        |v: &StructWithFixedArray| (v.ids, v.tail)
    );

    prop_case!(
        sandwiched_dynamic_members,
        SolSandwich,
        (u256_value(), dynamic_struct_value(), u256_value())
            .prop_map(|(head, inner, tail)| SandwichStruct { head, inner, tail }),
        |v: &SandwichStruct| (
            v.head,
            (v.inner.id, v.inner.name.clone(), v.inner.blob.clone()),
            v.tail
        )
    );

    prop_case!(
        structs_nested_two_levels_deep,
        SolDeep,
        (
            u256_value(),
            nested_struct_value(),
            proptest::collection::vec(text_value(), LENGTHS)
        )
            .prop_map(|(id, middle, names)| DeepStruct { id, middle, names }),
        |v: &DeepStruct| (
            v.id,
            (
                (v.middle.inner.a, v.middle.inner.b, v.middle.inner.c),
                v.middle.list.clone()
            ),
            v.names.clone()
        )
    );

    prop_case!(
        fixed_arrays_of_text,
        sol_data::FixedArray<sol_data::String, 3>,
        (text_value(), text_value(), text_value()).prop_map(|(a, b, c)| [a, b, c])
    );

    prop_case!(
        fixed_arrays_of_bytes,
        sol_data::FixedArray<sol_data::Bytes, 2>,
        (bytes_value(), bytes_value()).prop_map(|(a, b)| [a, b])
    );

    prop_case!(
        fixed_arrays_of_dynamic_arrays,
        sol_data::FixedArray<sol_data::Array<sol_data::Uint<256>>, 2>,
        (
            proptest::collection::vec(u256_value(), LENGTHS),
            proptest::collection::vec(u256_value(), LENGTHS)
        )
            .prop_map(|(a, b)| [a, b])
    );

    prop_case!(
        arrays_of_arrays_of_text,
        sol_data::Array<sol_data::Array<sol_data::String>>,
        proptest::collection::vec(proptest::collection::vec(text_value(), LENGTHS), LENGTHS)
    );

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

        /// Packed mode, checked against bytes derived from the specification rather than against
        /// alloy - alloy left-pads array elements with zeros, which is wrong for signed and for
        /// `bytesN` (see `packed_spec_case!`). The scalar in front puts the array at an offset
        /// that is not a whole number of words, which is where the element encoder's own
        /// alignment used to push the array forward.
        #[test]
        fn packed_arrays_start_where_the_preceding_scalar_ends(
            head in any::<u8>(),
            elements in any::<[u16; 3]>(),
        ) {
            let value = (head, elements);

            let mut buf = BytesMut::new();
            fluentbase_codec::SolidityPackedABI::encode(&value, &mut buf, 0)
                .expect("encoding a well-formed value must succeed");

            // "The direct arguments of abi.encodePacked are encoded without padding, as long as
            // they are not arrays" / "array elements are padded, but still encoded in-place".
            let mut expected = alloc_expected(head, &elements);
            expected.shrink_to_fit();

            prop_assert_eq!(buf.to_vec(), expected);
        }
    }

    fn alloc_expected(head: u8, elements: &[u16]) -> Vec<u8> {
        let mut expected = vec![head];
        for element in elements {
            let mut word = [0u8; 32];
            word[30..].copy_from_slice(&element.to_be_bytes());
            expected.extend_from_slice(&word);
        }
        expected
    }
}
