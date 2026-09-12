use alloy_primitives::{aliases::U40, FixedBytes};
use bytes::BytesMut;
use fluentbase_codec::CompactABI;

#[test]
fn compact_fixed_bytes_vectors_keep_aligned_element_slots() {
    fn check<const N: usize>() {
        for count in [0, 1, 2, 3, 17] {
            let values: Vec<_> = (0..count)
                .map(|index| FixedBytes::<N>::from([index as u8 + 1; N]))
                .collect();
            let mut encoded = BytesMut::new();
            CompactABI::encode(&values, &mut encoded, 0).unwrap();

            // Construct the wire bytes independently: three little-endian header words,
            // followed by elements right-padded to four-byte boundaries.
            let width = N.div_ceil(4) * 4;
            let mut expected = Vec::new();
            expected.extend_from_slice(&(count as u32).to_le_bytes());
            expected.extend_from_slice(&12u32.to_le_bytes());
            expected.extend_from_slice(&((width * count) as u32).to_le_bytes());
            for value in &values {
                expected.extend_from_slice(value.as_slice());
                expected.resize(expected.len() + width - N, 0);
            }
            assert_eq!(encoded.as_ref(), expected, "width={N}, count={count}");
            assert_eq!(
                CompactABI::<Vec<FixedBytes<N>>>::decode(&encoded, 0).unwrap(),
                values
            );
        }
    }
    check::<5>();
    check::<11>();
    check::<17>();
    check::<31>();
}

#[test]
fn compact_narrow_integer_vectors_preserve_values_and_padding() {
    let values = vec![U40::from(123), U40::from(456)];
    let mut encoded = BytesMut::new();
    CompactABI::encode(&values, &mut encoded, 0).unwrap();
    let expected = [
        2, 0, 0, 0, 12, 0, 0, 0, 16, 0, 0, 0, // vector header
        123, 0, 0, 0, 0, 0, 0, 0, // first aligned U40
        200, 1, 0, 0, 0, 0, 0, 0, // second aligned U40
    ];
    assert_eq!(encoded.as_ref(), expected);
    assert_eq!(CompactABI::<Vec<U40>>::decode(&encoded, 0).unwrap(), values);
}
