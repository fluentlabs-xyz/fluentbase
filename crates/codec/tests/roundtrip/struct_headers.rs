use super::*;
use hashbrown::HashMap;

#[derive(Clone, Copy, Codec, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct Pair {
    a: u64,
    b: u64,
}

#[derive(Codec, Debug, PartialEq)]
struct Nested {
    pair: Pair,
    c: u64,
}

#[derive(Clone, Codec, Debug, Default, PartialEq)]
struct Dynamic {
    pair: Pair,
    data: Bytes,
}

const _: () = {
    assert!(<Pair as Encoder<BE, 32, true, false>>::HEADER_SIZE.is_multiple_of(32));
    assert!(<Nested as Encoder<BE, 32, true, false>>::HEADER_SIZE.is_multiple_of(32));
    assert!(<Dynamic as Encoder<BE, 32, true, false>>::HEADER_SIZE.is_multiple_of(32));
};

sol! {
    struct SolPair {
        uint64 a;
        uint64 b;
    }

    struct SolNested {
        SolPair pair;
        uint64 c;
    }

    struct SolDynamic {
        SolPair pair;
        bytes data;
    }
}

impl From<Pair> for SolPair {
    fn from(pair: Pair) -> Self {
        Self {
            a: pair.a,
            b: pair.b,
        }
    }
}

fn assert_parity<T>(value: &T, expected: &[u8])
where
    T: Encoder<BE, 32, true, false> + core::fmt::Debug + PartialEq,
{
    let mut encoded = BytesMut::new();
    SolidityABI::encode(value, &mut encoded, 0).unwrap();
    assert_eq!(encoded.as_ref(), expected);
    assert_eq!(&SolidityABI::<T>::decode(&expected, 0).unwrap(), value);
}

#[test]
fn static_struct_header_matches_solidity_words() {
    assert_eq!(<Pair as Encoder<BE, 32, true, false>>::HEADER_SIZE, 64);
    assert_eq!(<Pair as Encoder<BE, 32, true, true>>::HEADER_SIZE, 64);
    assert_eq!(<Nested as Encoder<BE, 32, true, false>>::HEADER_SIZE, 96);
}

#[test]
fn static_struct_tuple_encode_matches_alloy() {
    let pair = Pair { a: 1, b: 2 };
    let expected = (SolPair::from(pair), 3u64).abi_encode();
    assert_parity(&(pair, 3u64), &expected);
}

#[test]
fn static_struct_tuple_decode_reads_trailing_argument() {
    let pair = Pair { a: 1, b: 2 };
    let expected = (SolPair::from(pair), 3u64).abi_encode();
    let decoded = SolidityABI::<(Pair, u64)>::decode(&expected.as_slice(), 0).unwrap();
    assert_eq!(decoded, (pair, 3));
}

#[test]
fn struct_function_arguments_match_alloy() {
    // Keep integer-padding coverage in FLU-1330; these values isolate head/tail placement.
    let values = [1, 2, u64::MAX / 2, u64::MAX];
    for a in values {
        for b in values {
            for c in values {
                for len in [0, 1, 32, 33] {
                    let pair = Pair { a, b };
                    let data = Bytes::from(vec![0xab; len]);
                    let expected = (SolPair::from(pair), c, data.clone()).abi_encode_params();
                    let args = (pair, c, data);
                    let mut encoded = BytesMut::new();
                    SolidityABI::encode_function_args(&args, &mut encoded).unwrap();
                    assert_eq!(encoded.as_ref(), expected);
                    assert_eq!(
                        SolidityABI::<(Pair, u64, Bytes)>::decode_function_args(
                            &expected.as_slice()
                        )
                        .unwrap(),
                        args
                    );
                }
            }
        }
    }
}

#[test]
fn nested_static_struct_matches_alloy_at_nonzero_offsets() {
    let value = Nested {
        pair: Pair { a: 1, b: u64::MAX },
        c: 3,
    };
    let expected = SolNested {
        pair: value.pair.into(),
        c: value.c,
    }
    .abi_encode();
    for offset in [0, 32, 64] {
        let mut encoded = BytesMut::from(vec![0xaa; offset].as_slice());
        SolidityABI::encode(&value, &mut encoded, offset).unwrap();
        assert_eq!(&encoded[..offset], vec![0xaa; offset]);
        assert_eq!(&encoded[offset..], expected);
        assert_eq!(
            SolidityABI::<Nested>::decode(&encoded, offset).unwrap(),
            value
        );
        assert_eq!(
            SolidityABI::<Nested>::partial_decode(&encoded, offset).unwrap(),
            (offset, 96)
        );
    }
}

#[test]
fn static_struct_vectors_match_alloy() {
    let pairs = [
        Pair { a: 1, b: 2 },
        Pair { a: 3, b: 4 },
        Pair { a: 5, b: 6 },
    ];
    for len in 0..=pairs.len() {
        let value = pairs[..len].to_vec();
        let expected = value
            .iter()
            .copied()
            .map(SolPair::from)
            .collect::<Vec<_>>()
            .abi_encode();
        assert_parity(&value, &expected);
    }
}

#[test]
fn dynamic_struct_vectors_match_alloy() {
    let value = vec![
        Dynamic {
            pair: Pair { a: 1, b: 2 },
            data: Bytes::from_static(b"first"),
        },
        Dynamic {
            pair: Pair { a: 3, b: 4 },
            data: Bytes::from_static(b"second"),
        },
    ];
    let expected = value
        .iter()
        .map(|v| SolDynamic {
            pair: v.pair.into(),
            data: v.data.clone(),
        })
        .collect::<Vec<_>>()
        .abi_encode();
    assert_parity(&value, &expected);
}

#[test]
fn static_struct_vectors_reject_truncated_heads() {
    let expected = vec![SolPair { a: 1, b: 2 }, SolPair { a: 3, b: 4 }].abi_encode();
    SolidityABI::<Vec<Pair>>::decode(&&expected[..expected.len() - 1], 0)
        .expect_err("both complete struct heads must fit in the array body");
}

#[test]
fn static_struct_map_keys_and_values_preserve_fields() {
    let value = HashMap::from([
        (Pair { a: 1, b: 2 }, Pair { a: 3, b: 4 }),
        (Pair { a: 5, b: 6 }, Pair { a: 7, b: 8 }),
    ]);
    let mut encoded = BytesMut::new();
    SolidityABI::encode(&value, &mut encoded, 0).unwrap();
    // The map's custom envelope contains sorted key/value arrays, each with full struct words.
    let expected = [32u64, 2, 64, 192, 2, 1, 2, 5, 6, 2, 3, 4, 7, 8]
        .into_iter()
        .flat_map(|word| U256::from(word).abi_encode())
        .collect::<Vec<_>>();
    assert_eq!(encoded.as_ref(), expected);
    assert_eq!(
        SolidityABI::<HashMap<Pair, Pair>>::decode(&encoded, 0).unwrap(),
        value
    );
}

#[test]
fn compact_and_packed_static_struct_layouts_are_preserved() {
    let value = Nested {
        pair: Pair { a: 1, b: 2 },
        c: 3,
    };
    let mut compact = BytesMut::new();
    CompactABI::encode(&value, &mut compact, 0).unwrap();
    let expected = [1u64, 2, 3]
        .into_iter()
        .flat_map(u64::to_le_bytes)
        .collect::<Vec<_>>();
    assert_eq!(compact.as_ref(), expected);
    assert_eq!(CompactABI::<Nested>::decode(&compact, 0).unwrap(), value);

    let mut packed = BytesMut::new();
    SolidityPackedABI::encode(&value, &mut packed, 0).unwrap();
    let expected = [1u64, 2, 3]
        .into_iter()
        .flat_map(u64::to_be_bytes)
        .collect::<Vec<_>>();
    assert_eq!(packed.as_ref(), expected);
    assert_eq!(
        SolidityPackedABI::<Nested>::decode(&packed, 0).unwrap(),
        value
    );
}
