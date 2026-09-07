//! `HashMap` and `HashSet` in Solidity mode - the one part of this codec that is not the
//! Solidity ABI.
//!
//! The specification has no mapping type, so there is no oracle and no external implementation to
//! agree with: this is our own format, and these tests are its definition. The layout they pin is
//! written out in `crates/codec/README.md`; if you change one, change the other.
//!
//! The extension cannot reach the wire. `rust_to_sol` rejects every generic type except `Vec` and
//! `FixedBytes`, so a map can never be a parameter or a return value of a contract method - the
//! router macro refuses to compile it. It is reachable only from a direct `SolidityABI::encode`
//! call inside a contract, for serialising something the contract itself will read back. That is
//! what keeps "Solidity mode is the specification" true: everything that crosses the contract
//! boundary is a specification type, and this format sits beside it rather than inside it.

use bytes::BytesMut;
use fluentbase_codec::{Encoder, SolidityABI};
use hashbrown::{HashMap, HashSet};

/// Words as they appear in the buffer, for readable expectations.
fn words(raw: &[u8]) -> Vec<String> {
    raw.chunks(32).map(hex::encode).collect()
}

fn word(value: u64) -> String {
    format!("{value:064x}")
}

/// The layout of a map, word by word.
///
/// A four-word head - the offset of the body, the entry count, then the offset of the keys region
/// and of the values region, each relative to its own slot - followed by the two regions, each
/// introduced by its own count.
#[test]
fn a_map_lays_out_its_keys_and_values_in_separate_regions() {
    let map: HashMap<u32, u32> = HashMap::from([(1, 100), (2, 200)]);
    let mut buf = BytesMut::new();
    SolidityABI::encode(&map, &mut buf, 0).expect("encoding a map");

    assert_eq!(
        words(&buf),
        vec![
            word(0x20), // body starts one word in
            word(2),    // entries
            word(0x40), // keys region, relative to this slot: 32 + 32 + 0x40 = 128
            word(0x80), // values region, relative to this slot: 32 + 64 + 0x80 = 224
            word(2),    // keys region: count
            word(1),
            word(2),
            word(2), // values region: count
            word(100),
            word(200),
        ],
        "map layout"
    );
}

/// The layout of a set: the same head minus the keys region.
#[test]
fn a_set_lays_out_one_region() {
    let set: HashSet<u32> = HashSet::from([7, 3, 5]);
    let mut buf = BytesMut::new();
    SolidityABI::encode(&set, &mut buf, 0).expect("encoding a set");

    assert_eq!(
        words(&buf),
        vec![
            word(0x20), // body starts one word in
            word(3),    // elements
            word(0x20), // data region, relative to this slot: 32 + 32 + 0x20 = 96
            word(3),    // data region: count
            word(3),    // sorted, not in insertion order
            word(5),
            word(7),
        ],
        "set layout"
    );
}

/// Entries are sorted by key before encoding, so the same contents always produce the same bytes.
///
/// This is the property the format exists for - it makes an encoding comparable and hashable - and
/// it is the one thing mutation testing cannot check, because no mutation operator breaks a
/// `sort_by`. Only a test can hold it.
#[test]
fn encoding_is_canonical_whatever_the_insertion_order() {
    let mut first: HashMap<u32, u32> = HashMap::new();
    for (key, value) in [(3u32, 30u32), (1, 10), (2, 20), (9, 90), (4, 40)] {
        first.insert(key, value);
    }
    let mut second: HashMap<u32, u32> = HashMap::new();
    for (key, value) in [(9u32, 90u32), (4, 40), (2, 20), (1, 10), (3, 30)] {
        second.insert(key, value);
    }

    let mut buf_first = BytesMut::new();
    let mut buf_second = BytesMut::new();
    SolidityABI::encode(&first, &mut buf_first, 0).expect("encoding the first map");
    SolidityABI::encode(&second, &mut buf_second, 0).expect("encoding the second map");
    assert_eq!(
        hex::encode(&buf_first[..]),
        hex::encode(&buf_second[..]),
        "two maps with the same entries must encode identically"
    );

    let mut set_first: HashSet<u32> = HashSet::new();
    let mut set_second: HashSet<u32> = HashSet::new();
    for value in [5u32, 1, 9, 3] {
        set_first.insert(value);
    }
    for value in [9u32, 3, 5, 1] {
        set_second.insert(value);
    }
    let mut buf_a = BytesMut::new();
    let mut buf_b = BytesMut::new();
    SolidityABI::encode(&set_first, &mut buf_a, 0).expect("encoding the first set");
    SolidityABI::encode(&set_second, &mut buf_b, 0).expect("encoding the second set");
    assert_eq!(
        hex::encode(&buf_a[..]),
        hex::encode(&buf_b[..]),
        "two sets with the same elements must encode identically"
    );
}

/// `partial_decode` answers the same question here as it does for a `Vec` in this mode: where the
/// length word sits, and how many entries are stored there.
#[test]
fn partial_decode_reports_the_length_word_and_the_entry_count() {
    for len in [0usize, 1, 2, 5] {
        let map: HashMap<u32, u32> = (0..len as u32)
            .map(|i| (i, i * 10))
            .collect::<HashMap<_, _>>();
        let mut buf = BytesMut::new();
        SolidityABI::encode(&map, &mut buf, 0).expect("encoding a map");
        assert_eq!(
            <HashMap<u32, u32> as Encoder<byteorder::BE, 32, true, false>>::partial_decode(&buf, 0)
                .expect("reading the map header"),
            (32, len),
            "map of {len} entries"
        );

        let set: HashSet<u32> = (0..len as u32).collect();
        let mut buf = BytesMut::new();
        SolidityABI::encode(&set, &mut buf, 0).expect("encoding a set");
        assert_eq!(
            <HashSet<u32> as Encoder<byteorder::BE, 32, true, false>>::partial_decode(&buf, 0)
                .expect("reading the set header"),
            (32, len),
            "set of {len} elements"
        );
    }
}

/// A map whose values are dynamic. The values region then holds offsets rather than the values
/// themselves, which is a different path through `write_bytes_solidity` than the flat case above,
/// and no test reached it.
#[test]
fn a_map_with_dynamic_values_round_trips() {
    for len in [0usize, 1, 3] {
        let map: HashMap<u32, String> = (0..len as u32)
            .map(|i| (i, "v".repeat(i as usize * 33)))
            .collect();
        let mut buf = BytesMut::new();
        SolidityABI::encode(&map, &mut buf, 0).expect("encoding a map of strings");
        let back: HashMap<u32, String> =
            SolidityABI::decode(&buf.clone().freeze(), 0).expect("decoding a map of strings");
        assert_eq!(back, map, "map of {len} string values");
    }

    // A dynamic key type as well, so both regions carry offsets.
    let keyed: HashMap<String, u32> = [("alpha", 1u32), ("beta", 2), ("gamma", 3)]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    let mut buf = BytesMut::new();
    SolidityABI::encode(&keyed, &mut buf, 0).expect("encoding a map with string keys");
    let back: HashMap<String, u32> =
        SolidityABI::decode(&buf.clone().freeze(), 0).expect("decoding a map with string keys");
    assert_eq!(back, keyed, "map with string keys");
}

/// Encoded somewhere other than the start of the buffer.
///
/// Every other test here writes at offset zero, where `buf.len() - offset` and `buf.len() + offset`
/// are the same number and the sign of that term cannot be wrong. Mutation testing pointed this
/// out: the offset arithmetic in both encoders survived every mutation until something wrote at a
/// non-zero position.
///
/// `partial_decode` is asserted here too: a map writes its body offset relative to its own head
/// while a `Vec` writes an absolute buffer position, so the map has to rebase on `offset` to report
/// the same thing. Read as absolute, it answered `(32, 32)` at a lead of 32 and `(32, 0)` at 64.
#[test]
fn maps_and_sets_encode_away_from_the_start_of_the_buffer() {
    for lead in [0usize, 32, 64, 96] {
        let map: HashMap<u32, u32> = HashMap::from([(1, 100), (2, 200), (3, 300)]);
        let mut buf = BytesMut::zeroed(lead);
        SolidityABI::encode(&map, &mut buf, lead).expect("encoding a map at an offset");
        let back: HashMap<u32, u32> =
            SolidityABI::decode(&buf.clone().freeze(), lead).expect("decoding a map at an offset");
        assert_eq!(back, map, "map written at {lead}");
        assert_eq!(
            <HashMap<u32, u32> as Encoder<byteorder::BE, 32, true, false>>::partial_decode(
                &buf, lead
            )
            .expect("reading the map header"),
            (lead + 32, 3),
            "map header at {lead}"
        );

        let set: HashSet<u32> = HashSet::from([7, 3, 5]);
        let mut buf = BytesMut::zeroed(lead);
        SolidityABI::encode(&set, &mut buf, lead).expect("encoding a set at an offset");
        let back: HashSet<u32> =
            SolidityABI::decode(&buf.clone().freeze(), lead).expect("decoding a set at an offset");
        assert_eq!(back, set, "set written at {lead}");
        assert_eq!(
            <HashSet<u32> as Encoder<byteorder::BE, 32, true, false>>::partial_decode(&buf, lead)
                .expect("reading the set header"),
            (lead + 32, 3),
            "set header at {lead}"
        );
    }
}

/// A truncated encoding is an error, never a panic. The offsets a decoder follows here come out of
/// the buffer, so every one of them is attacker-controlled once the input is not ours.
#[test]
fn truncated_encodings_are_rejected() {
    let map: HashMap<u32, u32> = HashMap::from([(1, 100), (2, 200)]);
    let mut buf = BytesMut::new();
    SolidityABI::encode(&map, &mut buf, 0).expect("encoding a map");
    let full = buf.to_vec();

    // Decoding may fail anywhere; reading the header must fail as soon as the header itself is
    // incomplete, and saying so is the point - a guard that never fires reads the same as one that
    // is not there.
    let map_header = 32 * 4;
    for cut in (0..full.len()).step_by(32) {
        let raw = bytes::Bytes::from(full[..cut].to_vec());
        let _ = SolidityABI::<HashMap<u32, u32>>::decode(&raw, 0);

        let header =
            <HashMap<u32, u32> as Encoder<byteorder::BE, 32, true, false>>::partial_decode(&raw, 0);
        if cut < map_header {
            assert!(
                header.is_err(),
                "a {cut}-byte buffer is shorter than the {map_header}-byte header, but the header \
                 was read as {header:?}"
            );
        }
    }

    let set: HashSet<u32> = HashSet::from([7, 3, 5]);
    let mut buf = BytesMut::new();
    SolidityABI::encode(&set, &mut buf, 0).expect("encoding a set");
    let full = buf.to_vec();

    let set_header = 32 * 3;
    for cut in (0..full.len()).step_by(32) {
        let raw = bytes::Bytes::from(full[..cut].to_vec());
        let _ = SolidityABI::<HashSet<u32>>::decode(&raw, 0);

        let header =
            <HashSet<u32> as Encoder<byteorder::BE, 32, true, false>>::partial_decode(&raw, 0);
        if cut < set_header {
            assert!(
                header.is_err(),
                "a {cut}-byte buffer is shorter than the {set_header}-byte header, but the header \
                 was read as {header:?}"
            );
        }
    }
}

/// Round trips, including the shapes that historically separate working implementations from lucky
/// ones: empty, one entry, and a dynamic value type.
#[test]
fn maps_and_sets_round_trip() {
    for len in [0usize, 1, 2, 7] {
        let map: HashMap<u32, u32> = (0..len as u32).map(|i| (i, i * 7)).collect();
        let mut buf = BytesMut::new();
        SolidityABI::encode(&map, &mut buf, 0).expect("encoding a map");
        let back: HashMap<u32, u32> =
            SolidityABI::decode(&buf.clone().freeze(), 0).expect("decoding a map");
        assert_eq!(back, map, "map of {len} entries");

        let set: HashSet<u32> = (0..len as u32).collect();
        let mut buf = BytesMut::new();
        SolidityABI::encode(&set, &mut buf, 0).expect("encoding a set");
        let back: HashSet<u32> =
            SolidityABI::decode(&buf.clone().freeze(), 0).expect("decoding a set");
        assert_eq!(back, set, "set of {len} elements");
    }
}
