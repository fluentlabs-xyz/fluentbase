use alloy_primitives::{Address, I128, U128};
use byteorder::BigEndian;
use bytes::{Buf, BytesMut};
use fluentbase_codec::{CodecError, DecodingError, Encoder, SolidityABI, SolidityPackedABI};

#[test]
fn option_width_and_flag_preserve_checked_reads() {
    for value in [Some(0u32), Some(256), None] {
        let mut encoded = BytesMut::new();
        SolidityABI::encode(&value, &mut encoded, 0).unwrap();
        assert_eq!(encoded.len(), 64);
        let encoded = encoded.freeze();
        assert_eq!(
            SolidityABI::<Option<u32>>::decode(&encoded, 0).unwrap(),
            value
        );
        assert_eq!(
            SolidityABI::<Option<u32>>::partial_decode(&encoded, 0).unwrap(),
            (0, 64)
        );

        for split in [16, 32, 63] {
            let segmented = encoded.slice(..split).chain(encoded.slice(split..));
            assert!(SolidityABI::<Option<u32>>::decode(&segmented, 0).is_err());
            assert!(SolidityABI::<Option<u32>>::partial_decode(&segmented, 0).is_err());
        }
        assert!(SolidityABI::<Option<u32>>::decode(&encoded, usize::MAX).is_err());
        assert!(SolidityABI::<Option<u32>>::partial_decode(&encoded, usize::MAX).is_err());
    }
}

fn check_packed_scalar<T>(value: T, width: usize)
where
    T: Encoder<BigEndian, 1, true, true> + PartialEq + core::fmt::Debug,
{
    let mut encoded = BytesMut::new();
    SolidityPackedABI::encode(&value, &mut encoded, 0).unwrap();
    assert_eq!(encoded.len(), width);
    let encoded = encoded.freeze();
    assert_eq!(SolidityPackedABI::<T>::decode(&encoded, 0).unwrap(), value);
    let segmented = encoded.slice(..width - 1).chain(encoded.slice(width - 1..));
    assert!(matches!(
        SolidityPackedABI::<T>::decode(&segmented, 0),
        Err(CodecError::Decoding(DecodingError::BufferTooSmall { .. }))
    ));
}

#[test]
fn packed_scalars_use_their_width_and_reject_short_chunks() {
    check_packed_scalar(Address::repeat_byte(0x42), 20);
    check_packed_scalar(U128::MAX, 16);
    check_packed_scalar(I128::MIN, 16);
}

#[test]
fn packed_array_at_unaligned_offset_rejects_short_chunks() {
    let value = [7u32, 9];
    let mut encoded = BytesMut::from(&[0xa5][..]);
    SolidityPackedABI::encode(&value, &mut encoded, 1).unwrap();
    assert_eq!(encoded.len(), 65);
    let encoded = encoded.freeze();
    assert_eq!(
        SolidityPackedABI::<[u32; 2]>::decode(&encoded, 1).unwrap(),
        value
    );
    let segmented = encoded.slice(..33).chain(encoded.slice(33..));
    assert!(matches!(
        SolidityPackedABI::<[u32; 2]>::decode(&segmented, 1),
        Err(CodecError::Decoding(DecodingError::BufferTooSmall { .. }))
    ));
}
