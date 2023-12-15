use crate::{
    common::{change_sign_be, try_divide_close_numbers},
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
    let mut result = (0u64, 0u64, 0u64, 0u64);

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
                (a3, a2, a1, a0) = change_sign_be((a3, a2, a1, a0));
            }
            result.0 = a0;
            result.1 = a1;
            result.2 = a2;
            result.3 = a3;
        }
    } else if a3 == b3 && a2 == b2 && a1 == b1 && a0 == b0 {
        if a0 != 0 {
            if a_sign == b_sign {
                result.0 = 1;
            } else {
                result = (
                    U64_ALL_BITS_ARE_1,
                    U64_ALL_BITS_ARE_1,
                    U64_ALL_BITS_ARE_1,
                    U64_ALL_BITS_ARE_1,
                )
            };
        }
    } else {
        if a_sign {
            (a3, a2, a1, a0) = change_sign_be((a3, a2, a1, a0));
        }
        if b_sign {
            (b3, b2, b1, b0) = change_sign_be((b3, b2, b1, b0));
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
                let div_res = try_divide_close_numbers(
                    unsafe { a_bytes.as_mut_ptr().offset(a_pos_start as isize) },
                    a_pos_end - a_pos_start,
                    unsafe { b_bytes.as_mut_ptr().offset(b_pos_start as isize) },
                    b_bytes.len() - b_pos_start,
                );
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
            v.clone_from_slice(&res[24..32]);
            result.0 = u64::from_be_bytes(v);
            v.clone_from_slice(&res[16..24]);
            result.1 = u64::from_be_bytes(v);
            v.clone_from_slice(&res[8..16]);
            result.2 = u64::from_be_bytes(v);
            v.clone_from_slice(&res[0..8]);
            result.3 = u64::from_be_bytes(v);
        }
    }

    result
}
