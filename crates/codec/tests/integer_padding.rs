use alloy_sol_types::{sol, SolType};
use bytes::BytesMut;
use fluentbase_codec::SolidityABI;

macro_rules! test_integer_padding {
    ($name:ident, $sol_type:ty, $values:expr) => {
        #[test]
        fn $name() {
            for value in $values {
                let expected = <$sol_type>::abi_encode(&value);
                for (offset, mut buf) in
                    [(0, BytesMut::new()), (32, BytesMut::from(&[0xA5; 96][..]))]
                {
                    SolidityABI::encode(&value, &mut buf, offset).unwrap();
                    let encoded = &buf[offset..offset + 32];
                    assert_eq!(encoded, expected, "value {value}, offset {offset}");
                    assert_eq!(
                        <$sol_type>::abi_decode_validate(encoded).unwrap(),
                        value,
                        "value {value}, offset {offset}",
                    );
                    assert!(buf[..offset].iter().all(|byte| *byte == 0xA5));
                    assert!(buf[offset + 32..].iter().all(|byte| *byte == 0xA5));
                }
            }
        }
    };
}

test_integer_padding!(solidity_u16_padding, sol!(uint16), [0u16, 1, u16::MAX]);
test_integer_padding!(solidity_u32_padding, sol!(uint32), [0u32, 1, u32::MAX]);
test_integer_padding!(solidity_u64_padding, sol!(uint64), [0u64, 1, u64::MAX]);
test_integer_padding!(
    solidity_i16_padding,
    sol!(int16),
    [0i16, 1, -1, i16::MIN, i16::MAX]
);
test_integer_padding!(
    solidity_i32_padding,
    sol!(int32),
    [0i32, 1, -1, i32::MIN, i32::MAX]
);
test_integer_padding!(
    solidity_i64_padding,
    sol!(int64),
    [0i64, 1, -1, i64::MIN, i64::MAX]
);
