#[no_mangle]
fn arithmetic_mul(
    x1: u64,
    x2: u64,
    x3: u64,
    x4: u64,
    y1: u64,
    y2: u64,
    y3: u64,
    y4: u64,
) -> (u64, u64, u64, u64) {
    fn multiply_u64(a: u64, b: u64) -> (u64, u64) {
        let a_lo = a as u32 as u64;
        let a_hi = (a >> 32) as u32 as u64;
        let b_lo = b as u32 as u64;
        let b_hi = (b >> 32) as u32 as u64;

        let lo = a_lo.wrapping_mul(b_lo);
        let mid1 = a_lo.wrapping_mul(b_hi);
        let mid2 = a_hi.wrapping_mul(b_lo);
        let hi = a_hi.wrapping_mul(b_hi);

        let mid_sum = mid1.wrapping_add(mid2);
        let hi_carry = mid_sum < mid1 || lo > (u64::MAX - mid_sum);

        let hi_result = hi.wrapping_add((mid_sum >> 32) + if hi_carry { 1 } else { 0 });
        let lo_result = lo.wrapping_add((mid_sum & 0xFFFFFFFF) << 32);

        (hi_result, lo_result)
    }

    let mut result = [0u64; 4];
    let x = [x1, x2, x3, x4];
    let y = [y1, y2, y3, y4];

    for i in 0..3 {
        let mut carry = 0u64;
        let b = y[i];

        for j in 0..3 {
            if i + j < 4 {
                // Ensure not to go out of bounds
                let a = x[j];

                let (hi, low) = multiply_u64(a, b);

                let overflow = {
                    let existing_low = &mut result[i + j];
                    let (low, o) = low.overflowing_add(*existing_low);
                    *existing_low = low;
                    o
                };

                carry = {
                    if i + j < 3 {
                        let existing_hi = &mut result[i + j + 1];
                        let hi = hi + overflow as u64;
                        let (hi, o0) = hi.overflowing_add(carry);
                        let (hi, o1) = hi.overflowing_add(*existing_hi);
                        *existing_hi = hi;

                        (o0 | o1) as u64
                    } else {
                        overflow as u64
                    }
                };
            }
        }
    }

    result.into()
}

#[test]
fn test_arithmetic_mul() {
    use crate::test_helper::*;
    use ethereum_types::U256;

    let u256_x = U256::from_dec_str("60000000000000000000000000000000000000000000").unwrap();
    let u256_y = U256::from_dec_str("2000000000000000000000000000").unwrap();

    // split the U256 into 4 u64 values
    let (u64_x_0, u64_x_1, u64_x_2, u64_x_3) = split_u256(u256_x);
    let (u64_y_0, u64_y_1, u64_y_2, u64_y_3) = split_u256(u256_y);

    let (res_0, res_1, res_2, res_3) = arithmetic_mul(
        u64_x_0, u64_x_1, u64_x_2, u64_x_3, u64_y_0, u64_y_1, u64_y_2, u64_y_3,
    );

    println!("RES: {:?} ", combine_u64(res_0, res_1, res_2, res_3));

    assert_eq!(
        combine_u64(res_0, res_1, res_2, res_3),
        U256::from_dec_str(
            "120000000000000000000000000000000000000000000000000000000000000000000000"
        )
        .unwrap()
    );
}
