#[cfg(test)]
mod tests {
    #[cfg(feature = "arithmetic_mul")]
    use crate::arithmetic::mul::arithmetic_mul;
    use crate::test_utils::{u256_from_be_u64, u256_from_le_u64, u256_into_le_tuple};

    #[cfg(feature = "arithmetic_mul")]
    #[test]
    fn test_arithmetic_mul() {
        use ethereum_types::U256;

        let cases =
            [
                // (
                //     U256::from_dec_str("60000000000000000000000000000000000000000000").unwrap(),
                //     U256::from_dec_str("2000000000000000000000000000").unwrap(),
                //     U256::from_dec_str(
                //         "120000000000000000000000000000000000000000000000000000000000000000000000",
                //     )
                //     .unwrap(),
                // ),
                // // -1 2 -2
                // (
                //     U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639935").unwrap(),
                //     U256::from_dec_str("2").unwrap(),
                //     U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639934").unwrap(),
                // ),
                (
                    U256::from_dec_str("8").unwrap(),
                    U256::from_dec_str("4").unwrap(),
                    U256::from_dec_str("32").unwrap(),
                ),
            ];

        for case in &cases {
            let u256_x = case.0;
            let u256_y = case.1;

            // split the U256 into 4 u64 values
            let (u64_x_0, u64_x_1, u64_x_2, u64_x_3) = u256_into_le_tuple(u256_x);
            let (u64_y_0, u64_y_1, u64_y_2, u64_y_3) = u256_into_le_tuple(u256_y);

            let (res_0, res_1, res_2, res_3) = arithmetic_mul(
                u64_x_0, u64_x_1, u64_x_2, u64_x_3, u64_y_0, u64_y_1, u64_y_2, u64_y_3,
            );

            let res = u256_from_le_u64(res_0, res_1, res_2, res_3);
            assert_eq!(res, case.2);
        }
    }
}
