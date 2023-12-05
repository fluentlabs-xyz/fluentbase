use crate::test_utils::{u256_from_be_tuple, u256_into_le_tuple};
use ethereum_types::U256;
use log::debug;

pub fn test_binary_cases(
    func_to_test: fn(u64, u64, u64, u64, u64, u64, u64, u64) -> (u64, u64, u64, u64),
    cases: &[(&str, &str, &str)],
) {
    for case in cases {
        debug!("");
        debug!("a {:?}", case.0);
        debug!("b {:?}", case.1);
        debug!("a-b expected {:?}", case.2);

        let u256_a = U256::from_dec_str(case.0).unwrap();
        let u256_b = U256::from_dec_str(case.1).unwrap();
        let u256_expected = U256::from_dec_str(case.2).unwrap();
        let a = u256_into_le_tuple(u256_a);
        let b = u256_into_le_tuple(u256_b);
        let res_expected = u256_into_le_tuple(u256_expected);

        let res_tuple = func_to_test(a.0, a.1, a.2, a.3, b.0, b.1, b.2, b.3);
        let mut res = u256_from_be_tuple(&res_tuple);

        debug!("a {:?}", a);
        debug!("b {:?}", b);
        debug!("res_tuple {:?}", res_tuple);
        debug!("res_expected {:?}", res_expected);

        let mut res_be = vec![0u8; 32];
        res.to_big_endian(&mut res_be);
        debug!("res_be {:x?}", res_be);

        assert_eq!(u256_expected, res);
    }
}
