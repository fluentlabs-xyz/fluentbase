use crate::{
    common::try_divide_close_numbers,
    consts::{BYTE_MAX_VAL, U256_BYTES_COUNT, U64_ALL_BITS_ARE_1, U64_MSBIT_IS_1},
};

#[no_mangle]
pub fn arithmetic_sdiv(
    b0: u64,
    b1: u64,
    b2: u64,
    b3: u64,
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
) -> (u64, u64, u64, u64) {
    let a_sign = a3 & U64_MSBIT_IS_1 > 0;
    let b_sign = b3 & U64_MSBIT_IS_1 > 0;
    let mut result = [0u64, 0u64, 0u64, 0u64];

    let mut a0 = a0;
    let mut a1 = a1;
    let mut a2 = a2;
    let mut a3 = a3;
    let mut b0 = b0;
    let mut b1 = b1;
    let mut b2 = b2;
    let mut b3 = b3;

    if b3 == 0 && b2 == 0 && b1 == 0 && (b0 == 1 || b0 == 0)
        || b3 == U64_ALL_BITS_ARE_1
            && b2 == U64_ALL_BITS_ARE_1
            && b1 == U64_ALL_BITS_ARE_1
            && b0 == U64_ALL_BITS_ARE_1
    {
        if b0 != 0 {
            if b0 == U64_ALL_BITS_ARE_1 && a_sign && b_sign {
                if a0 == 0 {
                    a0 = U64_ALL_BITS_ARE_1;
                    if a1 == 0 {
                        a1 = U64_ALL_BITS_ARE_1;
                        if a2 == 0 {
                            a2 = U64_ALL_BITS_ARE_1;
                            a0 -= 1;
                        } else {
                            a2 -= 1;
                        }
                    } else {
                        a1 -= 1;
                    }
                } else {
                    a0 -= 1;
                }
                a0 = !a0;
                a1 = !a1;
                a2 = !a2;
                a3 = !a3;
            }
            result[0] = a0;
            result[1] = a1;
            result[2] = a2;
            result[3] = a3;
        }
    } else if a3 == b3 && a2 == b2 && a1 == b1 && a0 == b0 {
        if a0 != 0 {
            if a_sign == b_sign {
                result[0] = 1;
            } else {
                for i in 0..4 {
                    result[i] = U64_ALL_BITS_ARE_1
                }
            };
        }
    } else {
        if a_sign {
            if a0 <= 0 {
                a0 = U64_MSBIT_IS_1;

                if a1 < 1 {
                    a1 = U64_MSBIT_IS_1;
                    if a2 < 1 {
                        a2 = U64_MSBIT_IS_1;
                        if a3 < 1 {
                            a3 = U64_MSBIT_IS_1;
                        } else {
                            a3 -= 1;
                        }
                    } else {
                        a2 -= 1;
                    }
                } else {
                    a1 -= 1;
                }
            } else {
                a0 -= 1;
            }
            a3 = !a3;
            a2 = !a2;
            a1 = !a1;
            a0 = !a0;
        }
        if b_sign {
            if b0 <= 0 {
                b0 = U64_MSBIT_IS_1;

                if b1 < 1 {
                    b1 = U64_MSBIT_IS_1;
                    if b2 < 1 {
                        b2 = U64_MSBIT_IS_1;
                        if b3 < 1 {
                            b3 = U64_MSBIT_IS_1;
                        } else {
                            b3 -= 1;
                        }
                    } else {
                        b2 -= 1;
                    }
                } else {
                    b1 -= 1;
                }
            } else {
                b0 -= 1;
            }
            b3 = !b3;
            b2 = !b2;
            b1 = !b1;
            b0 = !b0;
        }
        if a3 > b3
            || (a3 == b3 && a2 > b2)
            || (a3 == b3 && a2 == b2 && a1 > b1)
            || (a3 == b3 && a2 == b2 && a1 == b1 && a0 > b0)
        {
            let mut res = &mut [0u8; U256_BYTES_COUNT as usize];
            let mut res_vec = [0u8; U256_BYTES_COUNT as usize];
            let mut res_vec_idx: usize = 0;
            let mut a_bytes = &mut [0u8; U256_BYTES_COUNT as usize];
            let mut b_bytes = &mut [0u8; U256_BYTES_COUNT as usize];

            for i in 0..8 {
                a_bytes[i] = a3.to_be_bytes().as_slice()[i];
                b_bytes[i] = b3.to_be_bytes().as_slice()[i];
                a_bytes[i + 8] = a2.to_be_bytes().as_slice()[i];
                b_bytes[i + 8] = b2.to_be_bytes().as_slice()[i];
                a_bytes[i + 16] = a1.to_be_bytes().as_slice()[i];
                b_bytes[i + 16] = b1.to_be_bytes().as_slice()[i];
                a_bytes[i + 24] = a0.to_be_bytes().as_slice()[i];
                b_bytes[i + 24] = b0.to_be_bytes().as_slice()[i];
            }

            let mut a_pos_start: usize = 0;
            for i in 0..a_bytes.len() {
                if a_bytes[i] != 0 {
                    a_pos_start = i;
                    break;
                }
            }

            let mut b_pos_start = 0;
            for i in 0..U256_BYTES_COUNT as usize {
                if b_bytes[i] != 0 {
                    b_pos_start = i;
                    break;
                }
            }

            let mut a_pos_end = a_pos_start + b_bytes.len() - b_pos_start;
            loop {
                // debug!(
                //     "a_pos_start={} a_pos_end={} a_chunk({})={:x?} b_bytes({})={:x?}",
                //     a_pos_start,
                //     a_pos_end,
                //     a_bytes[a_pos_start..a_pos_end].len(),
                //     &a_bytes[a_pos_start..a_pos_end],
                //     &b_bytes[b_pos_start..].len(),
                //     &b_bytes[b_pos_start..],
                // );
                let div_res = try_divide_close_numbers(
                    unsafe { a_bytes.as_mut_ptr().offset(a_pos_start as isize) },
                    a_pos_end - a_pos_start,
                    unsafe { b_bytes.as_mut_ptr().offset(b_pos_start as isize) },
                    b_bytes.len() - b_pos_start,
                );
                // debug!(
                //     "a_chunk/b_bytes({}) = {:x?}",
                //     &a_bytes[a_pos_start..a_pos_end].len(),
                //     &a_bytes[a_pos_start..a_pos_end],
                // );
                // debug!("div_res={:?}\n\n", div_res);
                let res_vec_ptr = res_vec.as_mut_ptr();
                unsafe {
                    *res_vec_ptr.offset(res_vec_idx as isize) = div_res;
                }
                res_vec_idx += 1;

                a_pos_end += 1;
                if div_res > 0 {
                    for i in a_pos_start..a_bytes.len() {
                        if a_bytes[i] != 0 {
                            break;
                        }
                        a_pos_start += 1
                    }
                }

                if a_pos_end > a_bytes.len() {
                    break;
                }
            }
            let res_len = res.len();
            let res_ptr: *mut u8 = res.as_mut_ptr();
            let res_vec_ptr = res_vec.as_ptr();
            for i in 0..res_vec_idx {
                unsafe {
                    *res_ptr.offset((res_len - res_vec_idx + i) as isize) =
                        *res_vec_ptr.offset(i as isize);
                }
            }
            // println!("res {:?} \n\n", res);

            if a_sign != b_sign {
                let mut carry = true;
                for i in (0..res.len()).rev() {
                    res[i] = !res[i];
                    if carry {
                        if res[i] == BYTE_MAX_VAL as u8 {
                            res[i] = 0;
                        } else {
                            res[i] += 1;
                            carry = false;
                        };
                    }
                }
            }
            let mut v = [0u8; 8];
            for i in 0..4 {
                v.clone_from_slice(&res[24 - i * 8..32 - i * 8]);
                result[i] = u64::from_be_bytes(v);
            }
        }
    }

    (result[0], result[1], result[2], result[3])
}
