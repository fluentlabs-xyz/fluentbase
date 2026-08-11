use super::*;

fn encode_codec<T>(value: T) -> Vec<u8>
where
    T: Encoder<BE, 32, true, false>,
{
    let mut buf = BytesMut::new();
    SolidityABI::encode(&value, &mut buf, 0).unwrap();
    buf.freeze().to_vec()
}

fn assert_word_matches_alloy<T, S>(value: T, alloy: S)
where
    T: Encoder<BE, 32, true, false> + core::fmt::Debug + Copy,
    S: SolValue,
{
    let encoded = encode_codec(value);
    assert_eq!(
        hex::encode(&encoded),
        hex::encode(alloy.abi_encode()),
        "{value:?} does not match alloy",
    );
}

/// Zero is non-negative, so it pads with 0x00. A `> 0` sign test pads it with 0xFF instead and
/// yields a word that strict Solidity decoders and topic filters reject.
#[test]
fn unsigned_integers_match_alloy() {
    for v in [0u16, 1, u16::MAX] {
        assert_word_matches_alloy(v, v);
    }
    for v in [0u32, 1, u32::MAX] {
        assert_word_matches_alloy(v, v);
    }
    for v in [0u64, 1, u64::MAX] {
        assert_word_matches_alloy(v, v);
    }
}

#[test]
fn signed_integers_match_alloy() {
    for v in [0i16, 1, -1, i16::MIN, i16::MAX] {
        assert_word_matches_alloy(v, v);
    }
    for v in [0i32, 1, -1, i32::MIN, i32::MAX] {
        assert_word_matches_alloy(v, v);
    }
    for v in [0i64, 1, -1, i64::MIN, i64::MAX] {
        assert_word_matches_alloy(v, v);
    }
}

/// The padding runs inside structs and tuples too, which is where events and return values hit
/// it — a standalone-value test alone would not cover the path that ships.
#[test]
fn zero_integer_inside_a_struct_matches_alloy() {
    #[derive(Codec, Default, Debug, PartialEq)]
    struct Counters {
        recorded: u32,
        expected: u32,
        epoch: u64,
    }

    sol! {
        struct CountersSol {
            uint32 recorded;
            uint32 expected;
            uint64 epoch;
        }
    }

    let value = Counters {
        recorded: 0,
        expected: 5,
        epoch: 0,
    };
    let mut buf = BytesMut::new();
    SolidityABI::encode(&value, &mut buf, 0).unwrap();

    let alloy = CountersSol {
        recorded: 0,
        expected: 5,
        epoch: 0,
    };
    assert_eq!(
        hex::encode(buf.freeze()),
        hex::encode(alloy.abi_encode()),
        "zero fields inside a struct must pad with 0x00",
    );
}
