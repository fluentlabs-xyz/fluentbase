use alloy_sol_types::SolValue;
use byteorder::{BigEndian, ByteOrder, LittleEndian};
use bytes::{Buf, BytesMut};
use fluentbase_codec::{
    write_u32_aligned, Codec, CodecError, Encoder, SolidityABI, SolidityPackedABI,
};

#[test]
fn u8_uses_the_requested_offset_and_preserves_surrounding_bytes() {
    fn check<B: ByteOrder, const ALIGN: usize>() {
        let mut encoded = BytesMut::from(vec![0xa5; ALIGN + 2].as_slice());
        <u8 as Encoder<B, ALIGN, false, false>>::encode(&0x5c, &mut encoded, 1).unwrap();
        assert_eq!(encoded.len(), ALIGN + 2);
        assert_eq!(encoded[0], 0xa5);
        assert_eq!(encoded[ALIGN + 1], 0xa5);
        assert_eq!(
            <u8 as Encoder<B, ALIGN, false, false>>::decode(&encoded, 1).unwrap(),
            0x5c
        );
    }
    check::<LittleEndian, 4>();
    check::<BigEndian, 4>();
    check::<BigEndian, 32>();
}

#[derive(Debug, PartialEq, Codec)]
struct DynamicThenStatic {
    text: String,
    value: u32,
}

#[test]
fn derived_dynamic_members_use_the_same_stride_for_every_operation() {
    fn check<const ALIGN: usize>() {
        let value = DynamicThenStatic {
            text: "abc".into(),
            value: 0x12345678,
        };
        assert_eq!(
            <DynamicThenStatic as Encoder<BigEndian, ALIGN, true, false>>::HEADER_SIZE,
            32 + ALIGN
        );
        let mut encoded = BytesMut::new();
        let result = <DynamicThenStatic as Encoder<BigEndian, ALIGN, true, false>>::encode(
            &value,
            &mut encoded,
            0,
        );
        if ALIGN != 32 {
            assert!(matches!(result, Err(CodecError::Encoding(_))));
            assert!(encoded.is_empty());
            assert!(
                <DynamicThenStatic as Encoder<BigEndian, ALIGN, true, false>>::decode(&encoded, 0,)
                    .is_err()
            );
            assert!(
                <DynamicThenStatic as Encoder<BigEndian, ALIGN, true, false>>::partial_decode(
                    &encoded, 0,
                )
                .is_err()
            );
            return;
        }
        result.unwrap();
        assert_eq!(
            <DynamicThenStatic as Encoder<BigEndian, ALIGN, true, false>>::decode(&encoded, 0)
                .unwrap(),
            value
        );
    }
    check::<4>();
    check::<16>();
    check::<32>();
    check::<64>();
}

#[derive(Debug)]
struct OversizedStatic;

impl<B: ByteOrder, const ALIGN: usize, const STATIC: bool> Encoder<B, ALIGN, true, STATIC>
    for OversizedStatic
{
    const HEADER_SIZE: usize = 1;
    const IS_DYNAMIC: bool = false;

    fn encode(&self, buf: &mut BytesMut, _offset: usize) -> Result<(), CodecError> {
        buf.extend_from_slice(&[0x42; 33]);
        Ok(())
    }

    fn decode(_buf: &impl Buf, _offset: usize) -> Result<Self, CodecError> {
        Ok(Self)
    }

    fn partial_decode(_buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        Ok((offset, 1))
    }
}

#[test]
fn packed_array_rejects_an_element_that_exceeds_its_declared_width() {
    let mut buf = BytesMut::from(&[0xa5][..]);
    let result = SolidityPackedABI::encode(&[OversizedStatic], &mut buf, 1);
    assert!(matches!(result, Err(CodecError::Encoding(_))));
}

fn check_dynamic_head<T>(value: &T, reference: &[u8])
where
    T: Encoder<BigEndian, 32, true, false> + PartialEq + core::fmt::Debug,
{
    assert!(T::IS_DYNAMIC);
    for offset in [0, 1, 32, 33, 96] {
        for len in [0, offset, offset + 32, offset + 96] {
            let mut encoded = BytesMut::from(vec![0xa5; len].as_slice());
            let body_at = len.max(offset + 32);
            let mut expected = encoded.clone();
            expected.resize(body_at, 0);
            write_u32_aligned::<BigEndian, 32>(&mut expected, offset, body_at as u32);
            expected.extend_from_slice(&reference[32..]);

            SolidityABI::encode(value, &mut encoded, offset).unwrap();
            assert_eq!(encoded, expected, "offset={offset}, len={len}");
            assert_eq!(SolidityABI::<T>::decode(&encoded, offset).unwrap(), *value);
        }
    }
}

#[test]
fn dynamic_tuples_reserve_their_head_before_appending_the_body() {
    let single = ("abc".to_string(),);
    check_dynamic_head(&single, &single.abi_encode());
    let pair = (7u32, "abc".to_string());
    check_dynamic_head(&pair, &pair.abi_encode());
    let nested = (8u32, pair.clone(), "tail".to_string());
    check_dynamic_head(&nested, &nested.abi_encode());
}

#[test]
fn derived_structs_reserve_their_head_before_appending_the_body() {
    let value = DynamicThenStatic {
        text: "abc".into(),
        value: 7,
    };
    check_dynamic_head(&value, &(value.text.clone(), value.value).abi_encode());
}
