use crate::consts::{U64_HALF_BITS_COUNT, U64_LOW_PART_MASK};

#[no_mangle]
pub fn arithmetic_mul(
    b0: u64,
    b1: u64,
    b2: u64,
    b3: u64,
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
) -> (u64, u64, u64, u64) {
    fn multiply_u64(a: u64, b: u64) -> (u64, u64) {
        let a_lo = a & U64_LOW_PART_MASK;
        let a_hi = a >> U64_HALF_BITS_COUNT;
        let b_lo = b & U64_LOW_PART_MASK;
        let b_hi = b >> U64_HALF_BITS_COUNT;

        let lo = a_lo.wrapping_mul(b_lo);
        let mid1 = a_lo.wrapping_mul(b_hi);
        let mid2 = a_hi.wrapping_mul(b_lo);
        let hi = a_hi.wrapping_mul(b_hi);

        let mid_sum = mid1.wrapping_add(mid2);
        let hi_carry = mid_sum < mid1 || lo > (u64::MAX - mid_sum);

        let hi_result =
            hi.wrapping_add((mid_sum >> U64_HALF_BITS_COUNT) + if hi_carry { 1 } else { 0 });
        let lo_result = lo.wrapping_add((mid_sum & U64_LOW_PART_MASK) << 32);

        (hi_result, lo_result)
    }

    let mut result = [0u64; 4];
    let x = [a3, a2, a1, a0];
    let y = [b3, b2, b1, b0];

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

    (result[0], result[1], result[2], result[3])
}
