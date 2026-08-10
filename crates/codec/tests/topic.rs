//! Indexed event topics, checked against `alloy-sol-types` as an independent implementation of
//! the Solidity ABI's indexed-parameter encoding, plus hand-written vectors for the shapes that
//! ordinary ABI encoding gets wrong (dynamic values, fixed arrays, structs and nesting).

use alloy_primitives::{keccak256, Address, Bytes, FixedBytes, B256, I256, U256};
use alloy_sol_types::{sol_data, EventTopic, SolType};
use fluentbase_codec::{encode_indexed_topic, Codec, IndexedTopic, SolidityEventTopic};

/// The topic a contract would put in the log: the word itself for value types, the hash of the
/// preimage for reference types.
fn topic<T: SolidityEventTopic>(value: &T) -> B256 {
    match encode_indexed_topic(value).expect("encode topic") {
        IndexedTopic::Word(word) => B256::new(word),
        IndexedTopic::Preimage(preimage) => keccak256(&preimage),
    }
}

/// The topic `alloy-sol-types` produces for the same value.
fn expected<S>(value: &S::RustType) -> B256
where
    S: SolType + EventTopic,
{
    S::encode_topic(value).0
}

fn preimage<T: SolidityEventTopic>(value: &T) -> Vec<u8> {
    match encode_indexed_topic(value).expect("encode topic") {
        IndexedTopic::Word(word) => word.to_vec(),
        IndexedTopic::Preimage(preimage) => preimage.to_vec(),
    }
}

#[test]
fn value_types_occupy_the_topic_word_directly() {
    let address = Address::repeat_byte(0xab);
    assert_eq!(topic(&address), expected::<sol_data::Address>(&address));
    assert!(matches!(
        encode_indexed_topic(&address).unwrap(),
        IndexedTopic::Word(_)
    ));

    assert_eq!(topic(&true), expected::<sol_data::Bool>(&true));

    let value = U256::from(0xdead_beef_u64);
    assert_eq!(topic(&value), expected::<sol_data::Uint<256>>(&value));

    // A narrow uint is left-padded into the word just like uint256.
    let small = alloy_primitives::aliases::U8::from(7u8);
    assert_eq!(topic(&small), expected::<sol_data::Uint<8>>(&7u8));
    assert_eq!(topic(&7u8), expected::<sol_data::Uint<8>>(&7u8));

    assert_eq!(topic(&7u64), expected::<sol_data::Uint<64>>(&7u64));

    // Negative values are sign-extended across the padding rather than zero-padded.
    let negative = I256::unchecked_from(-1234i64);
    assert_eq!(topic(&negative), expected::<sol_data::Int<256>>(&negative));

    let word = FixedBytes::<32>::repeat_byte(0x11);
    assert_eq!(topic(&word), expected::<sol_data::FixedBytes<32>>(&word));

    // bytesN is padded on the right, unlike every numeric type.
    let short = FixedBytes::<4>::new([1, 2, 3, 4]);
    assert_eq!(topic(&short), expected::<sol_data::FixedBytes<4>>(&short));
    assert_eq!(
        preimage(&short),
        hex::decode("0102030400000000000000000000000000000000000000000000000000000000").unwrap()
    );
}

/// Zero is the value the padding rule is easiest to get wrong on: it is not negative, so it must
/// be zero-padded, but a sign test written as `> 0` sends it down the sign-extension branch and
/// produces `0xff..ff0000`. Round-tripping through this codec cannot catch that - the decoder
/// ignores the padding - so every width is checked against `alloy-sol-types` instead.
#[test]
fn zero_is_zero_padded_in_every_integer_width() {
    assert_eq!(topic(&0u8), expected::<sol_data::Uint<8>>(&0u8));
    assert_eq!(topic(&0u16), expected::<sol_data::Uint<16>>(&0u16));
    assert_eq!(topic(&0u32), expected::<sol_data::Uint<32>>(&0u32));
    assert_eq!(topic(&0u64), expected::<sol_data::Uint<64>>(&0u64));
    assert_eq!(topic(&0i16), expected::<sol_data::Int<16>>(&0i16));
    assert_eq!(topic(&0i32), expected::<sol_data::Int<32>>(&0i32));
    assert_eq!(topic(&0i64), expected::<sol_data::Int<64>>(&0i64));
    assert_eq!(
        topic(&U256::ZERO),
        expected::<sol_data::Uint<256>>(&U256::ZERO)
    );
    assert_eq!(
        topic(&I256::ZERO),
        expected::<sol_data::Int<256>>(&I256::ZERO)
    );
    assert_eq!(topic(&false), expected::<sol_data::Bool>(&false));

    assert_eq!(preimage(&0u64), [0u8; 32]);
}

/// The sign rule itself, on both sides of zero, so a fix for the zero case cannot quietly drop
/// sign extension for negatives.
#[test]
fn padding_follows_the_sign_of_the_value() {
    assert_eq!(topic(&(-1i32)), expected::<sol_data::Int<32>>(&-1i32));
    assert_eq!(preimage(&(-1i32)), [0xffu8; 32]);

    assert_eq!(topic(&i64::MIN), expected::<sol_data::Int<64>>(&i64::MIN));
    assert_eq!(topic(&i64::MAX), expected::<sol_data::Int<64>>(&i64::MAX));
    assert_eq!(topic(&u64::MAX), expected::<sol_data::Uint<64>>(&u64::MAX));
}

/// Zero inside a container: the members are concatenated in place, so a wrongly padded member
/// changes the hash rather than one visible word.
#[test]
fn zero_members_keep_the_container_topic_correct() {
    let values = vec![0u32, 1, u32::MAX];
    assert_eq!(
        topic(&values),
        expected::<sol_data::Array<sol_data::Uint<32>>>(&values)
    );

    let fixed = [0u64, 7];
    assert_eq!(
        topic(&fixed),
        expected::<sol_data::FixedArray<sol_data::Uint<64>, 2>>(&fixed)
    );

    let mixed = (0u32, "x".to_string());
    assert_eq!(
        topic(&mixed),
        expected::<(sol_data::Uint<32>, sol_data::String)>(&(0u32, "x".to_string()))
    );
}

#[test]
fn indexed_string_hashes_its_raw_contents() {
    let value = "hello".to_string();

    // Not `keccak256(abi.encode("hello"))`: no offset word, no length word, no padding.
    assert_eq!(preimage(&value), b"hello");
    assert_eq!(topic(&value), keccak256("hello"));
    assert_eq!(topic(&value), expected::<sol_data::String>(&value));
}

#[test]
fn indexed_bytes_hashes_its_raw_contents() {
    let value = Bytes::from_static(&[1, 2, 3]);

    assert_eq!(preimage(&value), [1, 2, 3]);
    assert_eq!(topic(&value), keccak256([1, 2, 3]));
    assert_eq!(topic(&value), expected::<sol_data::Bytes>(&value));
}

#[test]
fn indexed_empty_bytes_hashes_nothing_at_all() {
    let value = Bytes::new();

    assert!(preimage(&value).is_empty());
    assert_eq!(topic(&value), keccak256([]));
    assert_eq!(topic(&value), expected::<sol_data::Bytes>(&value));
}

#[test]
fn indexed_dynamic_array_drops_its_length_prefix() {
    let values = vec![U256::from(1), U256::from(2)];

    // Ordinary ABI encoding would start with an offset and a length word; the topic preimage is
    // just the elements.
    assert_eq!(
        hex::encode(preimage(&values)),
        concat!(
            "0000000000000000000000000000000000000000000000000000000000000001",
            "0000000000000000000000000000000000000000000000000000000000000002",
        )
    );
    assert_eq!(
        topic(&values),
        expected::<sol_data::Array<sol_data::Uint<256>>>(&values)
    );
}

#[test]
fn indexed_empty_dynamic_array_hashes_an_empty_preimage() {
    let values: Vec<U256> = Vec::new();

    assert!(preimage(&values).is_empty());
    assert_eq!(
        topic(&values),
        expected::<sol_data::Array<sol_data::Uint<256>>>(&values)
    );
}

#[test]
fn indexed_fixed_array_is_hashed_rather_than_inlined() {
    let values = [Address::repeat_byte(1), Address::repeat_byte(2)];

    // A fixed array is static, so ordinary ABI encoding leaves it in place; as an indexed
    // parameter it is still a reference type and gets hashed.
    assert!(matches!(
        encode_indexed_topic(&values).unwrap(),
        IndexedTopic::Preimage(_)
    ));
    assert_ne!(topic(&values), B256::from_slice(&preimage(&values)[..32]));
    assert_eq!(
        topic(&values),
        expected::<sol_data::FixedArray<sol_data::Address, 2>>(&values)
    );
}

#[test]
fn array_members_that_are_dynamic_are_padded_in_place() {
    let values = vec!["a".to_string(), "bc".to_string()];

    // Each element is padded up to a word; the offsets ordinary ABI encoding would emit are
    // absent.
    assert_eq!(
        hex::encode(preimage(&values)),
        concat!(
            "6100000000000000000000000000000000000000000000000000000000000000",
            "6263000000000000000000000000000000000000000000000000000000000000",
        )
    );
    assert_eq!(
        topic(&values),
        expected::<sol_data::Array<sol_data::String>>(&values)
    );
}

#[test]
fn an_empty_string_member_still_occupies_a_word() {
    let values = vec![String::new(), "a".to_string()];

    assert_eq!(preimage(&values).len(), 64);
    assert_eq!(
        topic(&values),
        expected::<sol_data::Array<sol_data::String>>(&values)
    );
}

#[test]
fn nested_arrays_are_flattened_without_offsets() {
    let values = vec![
        vec![U256::from(1), U256::from(2)],
        vec![U256::from(3)],
        Vec::new(),
    ];

    assert_eq!(preimage(&values).len(), 96);
    assert_eq!(
        topic(&values),
        expected::<sol_data::Array<sol_data::Array<sol_data::Uint<256>>>>(&values)
    );
}

#[derive(Codec, Default, Debug, PartialEq)]
struct Point {
    x: U256,
    y: U256,
}

#[derive(Codec, Default, Debug, PartialEq)]
struct Label {
    id: U256,
    name: String,
    tags: Vec<Bytes>,
}

type SolPoint = (sol_data::Uint<256>, sol_data::Uint<256>);
type SolLabel = (
    sol_data::Uint<256>,
    sol_data::String,
    sol_data::Array<sol_data::Bytes>,
);

#[test]
fn indexed_static_struct_is_hashed_not_truncated_to_its_first_word() {
    let point = Point {
        x: U256::from(1),
        y: U256::from(2),
    };

    // The old encoding copied the first word, so every point sharing an `x` collided.
    assert_ne!(topic(&point), B256::left_padding_from(&[1]));
    assert_eq!(
        topic(&point),
        expected::<SolPoint>(&(U256::from(1), U256::from(2)))
    );
}

#[test]
fn indexed_dynamic_struct_inlines_its_members() {
    let label = Label {
        id: U256::from(9),
        name: "fluent".to_string(),
        tags: vec![Bytes::from_static(b"a"), Bytes::from_static(b"bb")],
    };

    // Every member sits in place, padded up to a word: no offset words, no length words.
    assert_eq!(
        hex::encode(preimage(&label)),
        concat!(
            "0000000000000000000000000000000000000000000000000000000000000009",
            "666c75656e740000000000000000000000000000000000000000000000000000",
            "6100000000000000000000000000000000000000000000000000000000000000",
            "6262000000000000000000000000000000000000000000000000000000000000",
        )
    );
    assert_eq!(
        topic(&label),
        expected::<SolLabel>(&(
            U256::from(9),
            "fluent".to_string(),
            vec![Bytes::from_static(b"a"), Bytes::from_static(b"bb")],
        ))
    );
}

#[derive(Codec, Default, Debug, PartialEq)]
struct Nested {
    point: Point,
    points: Vec<Point>,
}

#[test]
fn nested_structs_recurse_with_the_same_rule() {
    let nested = Nested {
        point: Point {
            x: U256::from(1),
            y: U256::from(2),
        },
        points: vec![
            Point {
                x: U256::from(3),
                y: U256::from(4),
            },
            Point {
                x: U256::from(5),
                y: U256::from(6),
            },
        ],
    };

    assert_eq!(preimage(&nested).len(), 6 * 32);
    assert_eq!(
        topic(&nested),
        expected::<(SolPoint, sol_data::Array<SolPoint>)>(&(
            (U256::from(1), U256::from(2)),
            vec![
                (U256::from(3), U256::from(4)),
                (U256::from(5), U256::from(6)),
            ],
        ))
    );
}

#[test]
fn tuples_follow_the_struct_rule() {
    let value = (U256::from(1), "x".to_string());

    assert_eq!(
        topic(&value),
        expected::<(sol_data::Uint<256>, sol_data::String)>(&(U256::from(1), "x".to_string()))
    );
}
