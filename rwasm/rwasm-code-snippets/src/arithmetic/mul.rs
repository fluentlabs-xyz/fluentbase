use crate::common::mul;

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
    mul(a0, a1, a2, a3, b0, b1, b2, b3)
    // fn multiply_u64(a: u64, b: u64) -> (u64, u64) {
    //     let a_lo = a & U64_LOW_PART_MASK;
    //     let a_hi = a >> U64_HALF_BITS_COUNT;
    //     let b_lo = b & U64_LOW_PART_MASK;
    //     let b_hi = b >> U64_HALF_BITS_COUNT;
    //
    //     let lo = a_lo * b_lo;
    //     let mid1 = a_lo * b_hi;
    //     let mid2 = a_hi * b_lo;
    //     let hi = a_hi * b_hi;
    //
    //     let (mid_sum, hi_carry) = mid1.overflowing_add(mid2);
    //     let hi_carry = (hi_carry as u64) << 32;
    //
    //     let lo_result = lo.overflowing_add(mid_sum << 32);
    //     let hi_result = hi + (mid_sum >> U64_HALF_BITS_COUNT) + hi_carry + lo_result.1 as u64;
    //
    //     (hi_result, lo_result.0)
    // }
    //
    // let mut res = [0u64; 4];
    // let av = [a0, a1, a2, a3];
    // let bv = [b0, b1, b2, b3];
    //
    // for i in 0..4 {
    //     let mut carry: u64 = 0;
    //     let b = bv[i];
    //
    //     for j in 0..4 {
    //         let res_cur_idx = i + j;
    //         if res_cur_idx < 4 {
    //             let a = av[j];
    //
    //             let (h, l) = multiply_u64(a, b);
    //
    //             let res_chunk = &mut res[res_cur_idx];
    //             let (l, o) = l.overflowing_add(*res_chunk);
    //             carry += o as u64;
    //             *res_chunk = l;
    //
    //             let res_next_idx = res_cur_idx + 1;
    //             if res_next_idx < 4 {
    //                 let res_chunk = &mut res[res_next_idx];
    //                 let (h, o) = h.overflowing_add(carry);
    //                 carry = o as u64;
    //                 let (h, o) = h.overflowing_add(*res_chunk);
    //                 carry += o as u64;
    //                 *res_chunk = h;
    //             }
    //         }
    //     }
    // }
    //
    // (res[0], res[1], res[2], res[3])
}
