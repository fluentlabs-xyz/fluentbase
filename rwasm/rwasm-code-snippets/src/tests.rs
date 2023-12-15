#[cfg(test)]
mod all_tests {
    #[cfg(feature = "arithmetic_addmod")]
    use crate::arithmetic::addmod::arithmetic_addmod;
    #[cfg(feature = "arithmetic_div")]
    use crate::arithmetic::div::arithmetic_div;
    #[cfg(feature = "arithmetic_exp")]
    use crate::arithmetic::exp::arithmetic_exp;
    #[cfg(feature = "arithmetic_mod")]
    use crate::arithmetic::mod_impl::arithmetic_mod;
    #[cfg(feature = "arithmetic_mul")]
    use crate::arithmetic::mul::arithmetic_mul;
    #[cfg(feature = "arithmetic_sdiv")]
    use crate::arithmetic::sdiv::arithmetic_sdiv;
    #[cfg(feature = "arithmetic_smod")]
    use crate::arithmetic::smod_impl::arithmetic_smod;
    use crate::{
        arithmetic::sub::arithmetic_sub,
        test_utils::{u256_from_le_u64, u256_into_le_tuple},
    };
    use ethereum_types::U256;
    use log::debug;

    #[cfg(feature = "arithmetic_div")]
    #[test]
    fn test_arithmetic_div() {
        use ethereum_types::U256;

        let cases =
            [
                (
                    U256::from_dec_str("1").unwrap(),
                    U256::from_dec_str("1").unwrap(),
                    U256::from_dec_str("1").unwrap(),
                ),
                (
                    U256::from_dec_str("100").unwrap(),
                    U256::from_dec_str("3").unwrap(),
                    U256::from_dec_str("33").unwrap(),
                ),
                (
                    U256::from_dec_str("7435975337204372045884698348644506485689312179").unwrap(),
                    U256::from_dec_str("11209492868993368627820").unwrap(),
                    U256::from_dec_str("663364116834674892573348").unwrap(),
                ),
                (
                    U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639935").unwrap(),
                    U256::from_dec_str("1").unwrap(),
                    U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639935").unwrap(),
                ),
                (
                    U256::from_dec_str("7435974974357315440444149655801156533965628720").unwrap(),
                    U256::from_dec_str("11209492868993368627820").unwrap(),
                    U256::from_dec_str("663364084465052033976996").unwrap(),
                ),
                (
                    U256::from_dec_str("7435974971505144583019866185828197133679666480").unwrap(),
                    U256::from_dec_str("11209492868993368627820").unwrap(),
                    U256::from_dec_str("663364084210609581427364").unwrap(),
                ),

                (
                    U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639935").unwrap(),
                    U256::from_dec_str("108555083659983933209597798445644913612440610624038028786991485007418559037439").unwrap(),
                    U256::from_dec_str("1").unwrap(),
                ),
                (
                    U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639935").unwrap(),
                    U256::from_dec_str("25242039884198893745110504204788847439224557211638529451980005572607").unwrap(),
                    U256::from_dec_str("4587271463").unwrap(),
                ),
                (
                    U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639935").unwrap(),
                    U256::from_dec_str("2").unwrap(),
                    U256::from_dec_str("57896044618658097711785492504343953926634992332820282019728792003956564819967").unwrap(),
                ),
                (
                    U256::from_dec_str("123231").unwrap(),
                    U256::from_dec_str("0").unwrap(),
                    U256::from_dec_str("0").unwrap(),
                ),
                (
                    U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639935").unwrap(),
                    U256::from_dec_str("0").unwrap(),
                    U256::from_dec_str("0").unwrap(),
                ),
            ];

        for case in &cases {
            let a = case.0;
            let b = case.1;

            let (a0, a1, a2, a3) = u256_into_le_tuple(a);
            let (b0, b1, b2, b3) = u256_into_le_tuple(b);

            let (res_0, res_1, res_2, res_3) = arithmetic_div(b0, b1, b2, b3, a0, a1, a2, a3);

            let res = u256_from_le_u64(res_0, res_1, res_2, res_3);
            let mut expected_be = [0; 32];
            case.2.to_big_endian(&mut expected_be);
            let mut res_be = [0; 32];
            res.to_big_endian(&mut res_be);
            if res != case.2 {
                debug!("case with error:");
                debug!("a=       {}", a);
                debug!("b=       {}", b);
                debug!("expected={} ({:?})", case.2, expected_be);
                debug!("res=     {} ({:?})", res, res_be);
            }
            assert_eq!(case.2, res);
        }
    }

    #[cfg(feature = "arithmetic_exp")]
    #[test]
    fn test_arithmetic_exp() {
        use ethereum_types::U256;

        let cases =
            [
                (
                    U256::from_dec_str("1").unwrap(),
                    U256::from_dec_str("0").unwrap(),
                    U256::from_dec_str("1").unwrap(),
                ),
                (
                    U256::from_dec_str("1").unwrap(),
                    U256::from_dec_str("1").unwrap(),
                    U256::from_dec_str("1").unwrap(),
                ),
                (
                    U256::from_dec_str("1").unwrap(),
                    U256::from_dec_str("43486284623783462873").unwrap(),
                    U256::from_dec_str("1").unwrap(),
                ),
                (
                    U256::from_dec_str("1").unwrap(),
                    U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639935").unwrap(),
                    U256::from_dec_str("1").unwrap(),
                ),

                (
                    U256::from_dec_str("0").unwrap(),
                    U256::from_dec_str("0").unwrap(),
                    U256::from_dec_str("1").unwrap(),
                ),
                (
                    U256::from_dec_str("0").unwrap(),
                    U256::from_dec_str("1").unwrap(),
                    U256::from_dec_str("0").unwrap(),
                ),
                (
                    U256::from_dec_str("0").unwrap(),
                    U256::from_dec_str("43486284623783462873").unwrap(),
                    U256::from_dec_str("0").unwrap(),
                ),
                (
                    U256::from_dec_str("0").unwrap(),
                    U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639935").unwrap(),
                    U256::from_dec_str("0").unwrap(),
                ),
                (
                    U256::from_dec_str("10").unwrap(),
                    U256::from_dec_str("2").unwrap(),
                    U256::from_dec_str("100").unwrap(),
                ),
                (
                    U256::from_dec_str("2").unwrap(),
                    U256::from_dec_str("2").unwrap(),
                    U256::from_dec_str("4").unwrap(),
                ),
                (
                    U256::from_dec_str("2").unwrap(),
                    U256::from_dec_str("3").unwrap(),
                    U256::from_dec_str("8").unwrap(),
                ),
                (
                    U256::from_dec_str("2").unwrap(),
                    U256::from_dec_str("211").unwrap(),
                    U256::from_dec_str("3291009114642412084309938365114701009965471731267159726697218048").unwrap(),
                ),
                (
                    U256::from_dec_str("2").unwrap(),
                    U256::from_dec_str("252").unwrap(),
                    U256::from_dec_str("7237005577332262213973186563042994240829374041602535252466099000494570602496").unwrap(),
                ),
                (
                    U256::from_dec_str("2").unwrap(),
                    U256::from_dec_str("255").unwrap(),
                    U256::from_dec_str("57896044618658097711785492504343953926634992332820282019728792003956564819968").unwrap(),
                ),
                // overflow
                (
                    U256::from_dec_str("2").unwrap(),
                    U256::from_dec_str("256").unwrap(),
                    U256::from_dec_str("0").unwrap(),
                ),
                (
                    U256::from_dec_str("3").unwrap(),
                    U256::from_dec_str("156").unwrap(),
                    U256::from_dec_str("269721605590607563262106870407286853611938890184108047911269431464974473521").unwrap(),
                ),
                // overflow
                (
                    U256::from_dec_str("3").unwrap(),
                    U256::from_dec_str("162").unwrap(),
                    U256::from_dec_str("80834961238236718194504923518224208429833466278574202887857831530053261556873").unwrap(),
                ),
                // overflow
                (
                    U256::from_dec_str("100").unwrap(),
                    U256::from_dec_str("100").unwrap(),
                    U256::from_dec_str("59041770658110225754900818312084884949620587934026984283048776718299468660736").unwrap(),
                ),
                // overflow
                (
                    U256::from_dec_str("100").unwrap(),
                    U256::from_dec_str("1000").unwrap(),
                    U256::from_dec_str("0").unwrap(),
                ),
            ];

        for case in &cases {
            let a = case.0;
            let b = case.1;
            let r = case.2;

            let (a0, a1, a2, a3) = u256_into_le_tuple(a);
            let (b0, b1, b2, b3) = u256_into_le_tuple(b);

            let (res_0, res_1, res_2, res_3) = arithmetic_exp(b0, b1, b2, b3, a0, a1, a2, a3);

            let res = u256_from_le_u64(res_0, res_1, res_2, res_3);
            let mut expected_be = [0; 32];
            r.to_big_endian(&mut expected_be);
            let mut res_be = [0; 32];
            res.to_big_endian(&mut res_be);
            if res != r {
                debug!("case with error:");
                debug!("a=       {}", a);
                debug!("b=       {}", b);
                debug!("expected={} ({:?})", r, expected_be);
                debug!("res=     {} ({:?})", res, res_be);
            }
            assert_eq!(r, res);
        }
    }

    #[cfg(feature = "arithmetic_sdiv")]
    #[test]
    fn test_arithmetic_sdiv() {
        use ethereum_types::U256;

        let cases = [
            (
                U256::from_dec_str("1").unwrap(),
                U256::from_dec_str("1").unwrap(),
                U256::from_dec_str("1").unwrap(),
            ),
            (
                U256::from_dec_str("100").unwrap(),
                U256::from_dec_str("3").unwrap(),
                U256::from_dec_str("33").unwrap(),
            ),
            (            // res= 0x000000000000000000000000000000000000000000008c790a73e76a20fb8aa4

                U256::from_dec_str("7435975337204372045884698348644506485689312179").unwrap(),
                U256::from_dec_str("11209492868993368627820").unwrap(),
                U256::from_dec_str("663364116834674892573348").unwrap(),
            ),
            (
                U256::from_dec_str("7435974974357315440444149655801156533965628720").unwrap(),
                U256::from_dec_str("11209492868993368627820").unwrap(),
                U256::from_dec_str("663364084465052033976996").unwrap(),
            ),
            (
                U256::from_dec_str("7435974971505144583019866185828197133679666480").unwrap(),
                U256::from_dec_str("11209492868993368627820").unwrap(),
                U256::from_dec_str("663364084210609581427364").unwrap(),
            ),
            // a=   -1 -1 1
            (
                U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639935").unwrap(),
                U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639935").unwrap(),
                U256::from_dec_str("1").unwrap(),
            ),

            // a=   -3494230947320957983274982734981728917359869856329843243 -1  3494230947320957983274982734981728917359869856329843243
            (
                U256::from_dec_str("115792089237316195423567490777740586895286709682905582310540224138056799796693").unwrap(),
                U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639935").unwrap(),
                U256::from_dec_str("3494230947320957983274982734981728917359869856329843243").unwrap(),
            ),

            // a=   -437492374473294798249823982364926349823658375 -33424235324234
            (
                U256::from_dec_str("115792089237316195423570985008687470360895511370842314215475219081563305981561").unwrap(),
                U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457583974488894315702").unwrap(),
                U256::from_dec_str("13089076540700816492949964227746").unwrap(),
            ),

            // a=   -3494230947320957983274982734981728917359869856329843243 1 -3494230947320957983274982734981728917359869856329843243
            (
                U256::from_dec_str("115792089237316195423567490777740586895286709682905582310540224138056799796693").unwrap(),
                U256::from_dec_str("1").unwrap(),
                U256::from_dec_str("115792089237316195423567490777740586895286709682905582310540224138056799796693").unwrap(),
            ),

            // a= -437492374473294798249823982364926349823658375 439479274 -995478968760867658686957743461017800
            (
                U256::from_dec_str("115792089237316195423570985008687470360895511370842314215475219081563305981561").unwrap(),
                U256::from_dec_str("439479274").unwrap(),
                U256::from_dec_str("115792089237316195423570985008687907853268989186671803171798897050169668622137").unwrap(),
            ),
            (
                U256::from_dec_str("123231").unwrap(),
                U256::from_dec_str("0").unwrap(),
                U256::from_dec_str("0").unwrap(),
            ),
            (
                U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639935").unwrap(),
                U256::from_dec_str("0").unwrap(),
                U256::from_dec_str("0").unwrap(),
            ),
        ];

        for case in &cases {
            let a = case.0;
            let b = case.1;

            let (a0, a1, a2, a3) = u256_into_le_tuple(a);
            let (b0, b1, b2, b3) = u256_into_le_tuple(b);

            let (res_0, res_1, res_2, res_3) = arithmetic_sdiv(b0, b1, b2, b3, a0, a1, a2, a3);

            let res = u256_from_le_u64(res_0, res_1, res_2, res_3);
            let mut expected_be = [0; 32];
            case.2.to_big_endian(&mut expected_be);
            let mut res_be = [0; 32];
            res.to_big_endian(&mut res_be);
            if res != case.2 {
                debug!("case with error:");
                debug!("a=       {}", a);
                debug!("b=       {}", b);
                debug!("expected={} ({:?})", case.2, expected_be);
                debug!("res=     {} ({:?})", res, res_be);
            }
            assert_eq!(case.2, res);
        }
    }

    #[cfg(feature = "arithmetic_sub")]
    #[test]
    fn test_arithmetic_sub() {
        let cases = [(
            U256::from_dec_str(
                "57896044618658097711785492504343953926634992332820282019728792003956564819968",
            )
            .unwrap(),
            U256::from_dec_str(
                "57896044618658097711785492504343953926634992332820282019728792003956564819967",
            )
            .unwrap(),
            U256::from_dec_str("1").unwrap(),
        )];

        for case in &cases {
            let a = case.0;
            let b = case.1;

            let (a0, a1, a2, a3) = u256_into_le_tuple(a);
            let (b0, b1, b2, b3) = u256_into_le_tuple(b);

            let (res_0, res_1, res_2, res_3) = arithmetic_sub(b0, b1, b2, b3, a0, a1, a2, a3);

            let res = u256_from_le_u64(res_0, res_1, res_2, res_3);
            let mut expected_be = [0; 32];
            case.2.to_big_endian(&mut expected_be);
            let mut res_be = [0; 32];
            res.to_big_endian(&mut res_be);
            if res != case.2 {
                debug!("case with error:");
                debug!("a=       {}", a);
                debug!("b=       {}", b);
                debug!("expected={} ({:?})", case.2, expected_be);
                debug!("res=     {} ({:?})", res, res_be);
            }
            assert_eq!(case.2, res);
        }
    }

    #[cfg(feature = "arithmetic_mod")]
    #[test]
    fn test_arithmetic_mod() {
        use ethereum_types::U256;

        let cases =
            [
                (
                    U256::from_dec_str("1").unwrap(),
                    U256::from_dec_str("2").unwrap(),
                    U256::from_dec_str("1").unwrap(),
                ),
                (
                    U256::from_dec_str("1").unwrap(),
                    U256::from_dec_str("1").unwrap(),
                    U256::from_dec_str("0").unwrap(),
                ),
                (
                    U256::from_dec_str("100").unwrap(),
                    U256::from_dec_str("3").unwrap(),
                    U256::from_dec_str("1").unwrap(),
                ),
                (
                    U256::from_dec_str("7435975337204372045884698348644506485689312179").
                unwrap(),     U256::from_dec_str("11209492868993368627820").
                unwrap(),     U256::from_dec_str("615931049874225970819").
                unwrap(), ),
                // -1 -1 0
                (
                    U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639935").unwrap(),
                    U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639935").unwrap(),
                    U256::from_dec_str("0").unwrap(),
                ),
                // -1 1 1
                (
                    U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639935").unwrap(),
                    U256::from_dec_str("1").unwrap(),
                    U256::from_dec_str("0").unwrap(),
                ),
            ];

        for case in &cases {
            let a = case.0;
            let b = case.1;
            let r = case.2;

            let (a0, a1, a2, a3) = u256_into_le_tuple(a);
            let (b0, b1, b2, b3) = u256_into_le_tuple(b);

            let (res_0, res_1, res_2, res_3) = arithmetic_mod(b0, b1, b2, b3, a0, a1, a2, a3);

            let res = u256_from_le_u64(res_0, res_1, res_2, res_3);
            let mut expected_be = [0; 32];
            r.to_big_endian(&mut expected_be);
            let mut res_be = [0; 32];
            res.to_big_endian(&mut res_be);
            if res != r {
                debug!("case with error:");
                debug!("a=       {}", a);
                debug!("b=       {}", b);
                debug!("expected={} ({:?})", r, expected_be);
                debug!("res=     {} ({:?})", res, res_be);
            }
            assert_eq!(r, res);
        }
    }

    #[cfg(feature = "arithmetic_addmod")]
    #[test]
    fn test_arithmetic_addmod() {
        use ethereum_types::U256;

        let cases = [
            (
                U256::from_dec_str("10").unwrap(),
                U256::from_dec_str("10").unwrap(),
                U256::from_dec_str("8").unwrap(),
                U256::from_dec_str("4").unwrap(),
            ),
            (
                U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639935").unwrap(),
                U256::from_dec_str("2").unwrap(),
                U256::from_dec_str("2").unwrap(),
                U256::from_dec_str("1").unwrap(),
            ),
        ];

        for case in &cases {
            let a = case.0;
            let b = case.1;
            let c = case.2;
            let r = case.3;

            let (a0, a1, a2, a3) = u256_into_le_tuple(a);
            let (b0, b1, b2, b3) = u256_into_le_tuple(b);
            let (c0, c1, c2, c3) = u256_into_le_tuple(c);

            let (res_0, res_1, res_2, res_3) =
                arithmetic_addmod(c0, c1, c2, c3, b0, b1, b2, b3, a0, a1, a2, a3);

            let res = u256_from_le_u64(res_0, res_1, res_2, res_3);
            let mut expected_be = [0; 32];
            r.to_big_endian(&mut expected_be);
            let mut res_be = [0; 32];
            res.to_big_endian(&mut res_be);
            if res != r {
                debug!("case with error:");
                debug!("a=       {}", a);
                debug!("b=       {}", b);
                debug!("expected={} ({:?})", r, expected_be);
                debug!("res=     {} ({:?})", res, res_be);
            }
            assert_eq!(r, res);
        }
    }

    #[cfg(feature = "arithmetic_smod")]
    #[test]
    fn test_arithmetic_smod() {
        use ethereum_types::U256;

        let cases =
            [
                (
                    U256::from_dec_str("1").unwrap(),
                    U256::from_dec_str("1").unwrap(),
                    U256::from_dec_str("0").unwrap(),
                ),
                (
                    U256::from_dec_str("100").unwrap(),
                    U256::from_dec_str("3").unwrap(),
                    U256::from_dec_str("1").unwrap(),
                ),
                (
                    U256::from_dec_str("7435975337204372045884698348644506485689312179").
                unwrap(),     U256::from_dec_str("11209492868993368627820").
                unwrap(),     U256::from_dec_str("615931049874225970819").
                unwrap(), ),
                // -1 -1 0
                (
                    U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639935").unwrap(),
                    U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639935").unwrap(),
                    U256::from_dec_str("0").unwrap(),
                ),
                // -1 1 0
                (
                    U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639935").unwrap(),
                    U256::from_dec_str("1").unwrap(),
                    U256::from_dec_str("0").unwrap(),
                ),
                // -8 -3 -2
                (
                    U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639928").unwrap(),
                    U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639933").unwrap(),
                    U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639934").unwrap(),
                ),
                // 11 -3 2
                (
                    U256::from_dec_str("11").unwrap(),
                    U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639933").unwrap(),
                    U256::from_dec_str("2").unwrap(),
                ),
                // -11 3 -2
                (
                    U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639925").unwrap(),
                    U256::from_dec_str("3").unwrap(),
                    U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639934").unwrap(),
                ),
            ];

        for case in &cases {
            let a = case.0;
            let b = case.1;

            let (a0, a1, a2, a3) = u256_into_le_tuple(a);
            let (b0, b1, b2, b3) = u256_into_le_tuple(b);

            let (res_0, res_1, res_2, res_3) = arithmetic_smod(b0, b1, b2, b3, a0, a1, a2, a3);

            let res = u256_from_le_u64(res_0, res_1, res_2, res_3);
            let mut expected_be = [0; 32];
            case.2.to_big_endian(&mut expected_be);
            let mut res_be = [0; 32];
            res.to_big_endian(&mut res_be);
            if res != case.2 {
                debug!("case with error:");
                debug!("a=       {}", a);
                debug!("b=       {}", b);
                debug!("expected={} ({:?})", case.2, expected_be);
                debug!("res=     {} ({:?})", res, res_be);
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
            // overflow mul
            (
                U256::from_dec_str("356811923176489970264571492362373784095686655").unwrap(),
                U256::from_dec_str("356811923176489970264571492362373784095686655").unwrap(),
                U256::from_dec_str(
                    "115792089237316195423570985008687194229423631685700034896472859260344938266625",
                )
                .unwrap(),
            ),
            // overflowing mul
            (
                U256::from_dec_str("95780971304118053647396689196894323976171194868039680").unwrap(),
                U256::from_dec_str("95780971304118053647396689196894323976171194868039680").unwrap(),
                U256::from_dec_str(
                    "115792089237316144001553568720999090510483029748437283328961855016136437923840",
                )
                .unwrap(),
            ),
            // overflowing mul
            (
                U256::from_dec_str("7236998675585915423409399128287131963803921590493563082079543837970346803200").unwrap(),
                U256::from_dec_str("7236998675585915423409399128287131963803921590493563082079543837970346803200").unwrap(),
                U256::from_dec_str("0").unwrap(),
            ),
            (
                U256::from_dec_str("7237005577332262213973186563042994240829374041602535252466099000494570602496").unwrap(),
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
