#[cfg(test)]
mod tests {
    #[cfg(feature = "arithmetic_mul")]
    use crate::arithmetic::div::arithmetic_div;
    #[cfg(feature = "arithmetic_mul")]
    use crate::arithmetic::mul::arithmetic_mul;
    use crate::test_utils::{u256_from_le_u64, u256_into_le_tuple};
    use log::debug;

    #[cfg(feature = "arithmetic_div")]
    #[test]
    fn test_arithmetic_div() {
        use ethereum_types::U256;

        let cases = [(
            U256::from_dec_str("1").unwrap(),
            U256::from_dec_str("1").unwrap(),
            U256::from_dec_str("1").unwrap(),
        )];

        for case in &cases {
            let a = case.0;
            let b = case.1;

            let (a0, a1, a2, a3) = u256_into_le_tuple(a);
            let (b0, b1, b2, b3) = u256_into_le_tuple(b);

            let (res_0, res_1, res_2, res_3) = arithmetic_div(a0, a1, a2, a3, b0, b1, b2, b3);

            let res = u256_from_le_u64(res_0, res_1, res_2, res_3);
            let mut expected_be = vec![0; 32];
            case.2.to_big_endian(&mut expected_be);
            let mut res_be = vec![0; 32];
            res.to_big_endian(&mut res_be);
            if res != case.2 {
                debug!("case with error:");
                debug!("a=       {}", a);
                debug!("b=       {}", b);
                debug!("expected={} ({:x?})", case.2, expected_be);
                debug!("res=     {} ({:x?})", res, res_be);
            }
            assert_eq!(case.2, res);
        }
    }

    #[cfg(feature = "arithmetic_mul")]
    #[test]
    fn test_arithmetic_mul() {
        use ethereum_types::U256;

        let cases = [
            (
                U256::from_dec_str("8").unwrap(),
                U256::from_dec_str("4").unwrap(),
                U256::from_dec_str("32").unwrap(),
            ),
            (
                U256::from_dec_str("170141183460469231731687303715884105728").unwrap(),
                U256::from_dec_str("170141183460469231731687303715884105728").unwrap(),
                U256::from_dec_str(
                    "28948022309329048855892746252171976963317496166410141009864396001978282409984",
                )
                .unwrap(),
            ),
            (
                U256::from_dec_str(
                    "7237005577332262213973186563042994240829374041602535252466099000494570602496",
                )
                .unwrap(),
                U256::from_dec_str("2").unwrap(),
                U256::from_dec_str(
                    "14474011154664524427946373126085988481658748083205070504932198000989141204992",
                )
                .unwrap(),
            ),
            (
                U256::from_dec_str("60000000000000000000000000000000000000000000").unwrap(),
                U256::from_dec_str("2000000000000000000000000000").unwrap(),
                U256::from_dec_str(
                    "120000000000000000000000000000000000000000000000000000000000000000000000",
                )
                .unwrap(),
            ),
            // -1 2 -2
            (
                U256::from_dec_str(
                    "115792089237316195423570985008687907853269984665640564039457584007913129639935",
                )
                .unwrap(),
                U256::from_dec_str("2").unwrap(),
                U256::from_dec_str(
                    "115792089237316195423570985008687907853269984665640564039457584007913129639934",
                )
                .unwrap(),
            ),
            // -1 -1 1
            (
                U256::from_dec_str(
                    "115792089237316195423570985008687907853269984665640564039457584007913129639935",
                )
                .unwrap(),
                U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639935").unwrap(),
                U256::from_dec_str("1")
                .unwrap(),
            ),
        ];

        for case in &cases {
            let a = case.0;
            let b = case.1;

            let (a0, a1, a2, a3) = u256_into_le_tuple(a);
            let (b0, b1, b2, b3) = u256_into_le_tuple(b);

            let (res_0, res_1, res_2, res_3) = arithmetic_mul(a0, a1, a2, a3, b0, b1, b2, b3);

            let res = u256_from_le_u64(res_0, res_1, res_2, res_3);
            let mut expected_be = vec![0; 32];
            case.2.to_big_endian(&mut expected_be);
            let mut res_be = vec![0; 32];
            res.to_big_endian(&mut res_be);
            if res != case.2 {
                debug!("case with error:");
                debug!("a=       {}", a);
                debug!("b=       {}", b);
                debug!("expected={} ({:x?})", case.2, expected_be);
                debug!("res=     {} ({:x?})", res, res_be);
            }
            assert_eq!(case.2, res);
        }
    }
}
