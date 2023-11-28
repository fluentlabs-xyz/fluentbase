use crate::consts::{U64_MAX_VAL, U64_MSBIT_IS_1};

#[no_mangle]
pub fn arithmetic_sub(
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
    b0: u64,
    b1: u64,
    b2: u64,
    b3: u64,
) -> (u64, u64, u64, u64) {
    let a0_sign: u64 = a0 & U64_MSBIT_IS_1;

    let mut borrow: u64 = 0;
    let mut s0: u64 = 0;
    let mut s1: u64 = 0;
    let mut s2: u64 = 0;
    let mut s3: u64 = 0;

    if a3 >= b3 {
        s3 = a3 - b3;
    } else {
        s3 = U64_MAX_VAL - b3 + a3 + (1 - borrow);
        borrow = 1;
    }

    if a2 >= b2 + borrow {
        s2 = a2 - b2 - borrow;
        borrow = 0;
    } else {
        s2 = U64_MAX_VAL - b2 + a2 + (1 - borrow);
        borrow = 1;
    }

    if a1 >= b1 + borrow {
        s1 = a1 - b1 - borrow;
        borrow = 0;
    } else {
        s1 = U64_MAX_VAL - b1 + a1 + (1 - borrow);
        borrow = 1;
    }

    if a0 >= b0 + borrow {
        s0 = a0 - b0 - borrow;
    } else {
        if a0_sign > 0 {
            // TODO process overflow
            s0 = U64_MSBIT_IS_1;
        } else {
            s0 = U64_MAX_VAL - b0 + a0 + (1 - borrow);
        }
    }

    (s0, s1, s2, s3)
}

#[cfg(test)]
mod tests {
    use crate::{
        arithmetic::sub::arithmetic_sub,
        test_helper::{combine256_tuple_be, split_u256_be},
    };
    use ethereum_types::U256;
    use log::debug;

    #[test]
    pub fn bitwise_sub() {
        // [(a,b,res), ...]
        let cases = [
            (
                "770000000000000000000000000000000000000000000",
                "3000000000000000000000000000000000000000",
                "769997000000000000000000000000000000000000000",
            ),
            ("1000", "777", "223"),
            // 0, 9, -9
            (
                "0",
                "9",
                "115792089237316195423570985008687907853269984665640564039457584007913129639927",
            ),
            // -9, -9, 0
            (
                "115792089237316195423570985008687907853269984665640564039457584007913129639927",
                "115792089237316195423570985008687907853269984665640564039457584007913129639927",
                "0",
            ),
            // -9, 9, -18
            (
                "115792089237316195423570985008687907853269984665640564039457584007913129639927",
                "9",
                "115792089237316195423570985008687907853269984665640564039457584007913129639918",
            ),
            (
                "1579684469184883832639141051588852743717413708303648619256214454273",
                "200890243713775611099931966610046929622137341824116535096625",
                "1579684268294640118863529951656886133670484086166306795139679357648",
            ),
            (
                "94179513839427778125920318524484559090418146069193034825729",
                "22988181401826657186705067401604306861967601267430683953",
                "94156525658025951468733613457082954783556178467925604141776",
            ),
            // 0x000000000000000f000000000000000000000000000000000000000000000000
            // 0x0000000000000000000000000000000100000000000000000000000000000000
            // 0x000000000000000effffffffffffffff00000000000000000000000000000000
            (
                "94156526030800211457536841348114996241535331666960517693440",
                "340282366920938463463374607431768211456",
                "94156526030800211457196558981194057778071957059528749481984",
            ),
        ];

        for case in &cases {
            debug!("");
            debug!("a {:?}", case.0);
            debug!("b {:?}", case.1);
            debug!("a-b expected {:?}", case.2);

            let u256_a = U256::from_dec_str(case.0).unwrap();
            let u256_b = U256::from_dec_str(case.1).unwrap();
            let u256_expected = U256::from_dec_str(case.2).unwrap();
            let a = split_u256_be(u256_a);
            let b = split_u256_be(u256_b);
            let res_expected = split_u256_be(u256_expected);

            let res_tuple = arithmetic_sub(a.0, a.1, a.2, a.3, b.0, b.1, b.2, b.3);
            let mut res = combine256_tuple_be(&res_tuple);

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
}
