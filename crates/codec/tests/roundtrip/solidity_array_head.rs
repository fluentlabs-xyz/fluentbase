use super::*;

#[derive(Codec, Default, Debug, Clone, PartialEq)]
struct DynElem {
    blob: Bytes,
    tag: FixedBytes<32>,
    epoch: u64,
}

sol! {
    struct DynElemSol {
        bytes blob;
        bytes32 tag;
        uint64 epoch;
    }
}

#[derive(Codec, Default, Debug, Clone, PartialEq)]
struct StaticWide {
    who: Address,
    hash: FixedBytes<32>,
    epoch: u64,
}

sol! {
    struct StaticWideSol {
        address who;
        bytes32 hash;
        uint64 epoch;
    }
}

fn dyn_elem(i: u8) -> DynElem {
    DynElem {
        blob: Bytes::from(vec![0xA0 + i; 3]),
        tag: FixedBytes::<32>::from([0xB0 + i; 32]),
        epoch: 0x1100 + i as u64,
    }
}

fn dyn_elem_sol(i: u8) -> DynElemSol {
    DynElemSol {
        blob: alloy_primitives::Bytes::from(vec![0xA0 + i; 3]),
        tag: FixedBytes::<32>::from([0xB0 + i; 32]),
        epoch: 0x1100 + i as u64,
    }
}

fn static_wide(i: u8) -> StaticWide {
    StaticWide {
        who: Address::from([0x10 + i; 20]),
        hash: FixedBytes::<32>::from([0xB0 + i; 32]),
        epoch: 0x1100 + i as u64,
    }
}

fn static_wide_sol(i: u8) -> StaticWideSol {
    StaticWideSol {
        who: Address::from([0x10 + i; 20]),
        hash: FixedBytes::<32>::from([0xB0 + i; 32]),
        epoch: 0x1100 + i as u64,
    }
}

fn encode_codec<T>(values: &Vec<T>) -> Vec<u8>
where
    Vec<T>: Encoder<BE, 32, true, false>,
{
    let mut buf = BytesMut::new();
    SolidityABI::encode(values, &mut buf, 0).unwrap();
    buf.freeze().to_vec()
}

fn assert_matches_alloy(n: usize, codec: &[u8], alloy: &[u8]) {
    assert_eq!(
        codec.len(),
        alloy.len(),
        "n={n}: length differs from alloy\ncodec: {}\nalloy: {}",
        hex::encode(codec),
        hex::encode(alloy),
    );
    for (word, (got, want)) in codec.chunks(32).zip(alloy.chunks(32)).enumerate() {
        assert_eq!(
            got,
            want,
            "n={n}: word {word} (@{}) differs\ncodec: {}\nalloy: {}",
            word * 32,
            hex::encode(got),
            hex::encode(want),
        );
    }
}

/// A dynamic element gets one 32-byte offset word in the head; the previous encoder strode by
/// `HEADER_SIZE` (72 here) and wrote element 1's head into element 0's tail.
#[test]
fn dynamic_element_array_matches_alloy() {
    for n in 1..=4usize {
        let values: Vec<DynElem> = (0..n).map(|i| dyn_elem(i as u8)).collect();
        let alloy: Vec<DynElemSol> = (0..n).map(|i| dyn_elem_sol(i as u8)).collect();

        let encoded = encode_codec(&values);
        assert_matches_alloy(n, &encoded, &alloy.abi_encode());

        let decoded: Vec<DynElem> = SolidityABI::decode(&encoded.as_slice(), 0).unwrap();
        assert_eq!(decoded, values, "n={n}: codec round-trip");
    }
}

/// A static element wider than one word is inlined in the head, so its stride is the aligned
/// `HEADER_SIZE`. This only diverges from `ALIGN.max(HEADER_SIZE)` once three elements are
/// present — the derive's own `align_up` masks the gap at n=1 and n=2.
#[test]
fn static_wide_element_array_matches_alloy() {
    for n in 1..=4usize {
        let values: Vec<StaticWide> = (0..n).map(|i| static_wide(i as u8)).collect();
        let alloy: Vec<StaticWideSol> = (0..n).map(|i| static_wide_sol(i as u8)).collect();

        let encoded = encode_codec(&values);
        assert_matches_alloy(n, &encoded, &alloy.abi_encode());

        let decoded: Vec<StaticWide> = SolidityABI::decode(&encoded.as_slice(), 0).unwrap();
        assert_eq!(decoded, values, "n={n}: codec round-trip");
    }
}

#[test]
fn static_single_word_element_array_matches_alloy() {
    for n in 1..=4usize {
        let values: Vec<Address> = (0..n).map(|i| Address::from([0x20 + i as u8; 20])).collect();

        let encoded = encode_codec(&values);
        assert_matches_alloy(n, &encoded, &values.abi_encode());

        let decoded: Vec<Address> = SolidityABI::decode(&encoded.as_slice(), 0).unwrap();
        assert_eq!(decoded, values, "n={n}: codec round-trip");
    }
}

#[test]
fn decodes_alloy_encoded_arrays() {
    for n in 1..=4usize {
        let alloy_dyn: Vec<DynElemSol> = (0..n).map(|i| dyn_elem_sol(i as u8)).collect();
        let decoded: Vec<DynElem> =
            SolidityABI::decode(&alloy_dyn.abi_encode().as_slice(), 0).unwrap();
        let expected: Vec<DynElem> = (0..n).map(|i| dyn_elem(i as u8)).collect();
        assert_eq!(decoded, expected, "n={n}: alloy bytes -> codec (dynamic)");

        let alloy_wide: Vec<StaticWideSol> = (0..n).map(|i| static_wide_sol(i as u8)).collect();
        let decoded: Vec<StaticWide> =
            SolidityABI::decode(&alloy_wide.abi_encode().as_slice(), 0).unwrap();
        let expected: Vec<StaticWide> = (0..n).map(|i| static_wide(i as u8)).collect();
        assert_eq!(decoded, expected, "n={n}: alloy bytes -> codec (static wide)");
    }
}
