use crate::{
    common::{convert_sign_le, try_divide_close_numbers},
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
    let mut result_le = (0u64, 0u64, 0u64, 0u64);

    let mut a = (a0, a1, a2, a3);
    let mut b = (b0, b1, b2, b3);

    if b.3 == 0 && b.2 == 0 && b.1 == 0 && (b.0 == 1 || b.0 == 0)
        || b.3 == U64_ALL_BITS_ARE_1
            && b.2 == U64_ALL_BITS_ARE_1
            && b.1 == U64_ALL_BITS_ARE_1
            && b.0 == U64_ALL_BITS_ARE_1
    {
        if b.0 != 0 {
            if b.0 == U64_ALL_BITS_ARE_1 && a_sign && b_sign {
                a = convert_sign_le(a);
            }
            result_le = a;
        }
    } else if a.3 == b.3 && a.2 == b.2 && a.1 == b.1 && a.0 == b.0 {
        if a.0 != 0 {
            if a_sign == b_sign {
                result_le.0 = 1;
            } else {
                result_le = (
                    U64_ALL_BITS_ARE_1,
                    U64_ALL_BITS_ARE_1,
                    U64_ALL_BITS_ARE_1,
                    U64_ALL_BITS_ARE_1,
                )
            };
        }
    } else {
        if a_sign {
            a = convert_sign_le(a);
        }
        if b_sign {
            b = convert_sign_le(b);
        }
        if a.3 > b.3
            || (a.3 == b.3 && a.2 > b.2)
            || (a.3 == b.3 && a.2 == b.2 && a.1 > b.1)
            || (a.3 == b.3 && a.2 == b.2 && a.1 == b.1 && a.0 > b.0)
        {
            let mut res_be = &mut [0u8; U256_BYTES_COUNT as usize];
            let mut res_vec = [0u8; U256_BYTES_COUNT as usize];
            let mut res_vec_idx: usize = 0;
            let mut a_bytes = &mut [0u8; U256_BYTES_COUNT as usize];
            let mut b_bytes = &mut [0u8; U256_BYTES_COUNT as usize];

            for i in 0..8 {
                a_bytes[i] = a.3.to_be_bytes().as_slice()[i];
                b_bytes[i] = b.3.to_be_bytes().as_slice()[i];
                a_bytes[i + 8] = a.2.to_be_bytes().as_slice()[i];
                b_bytes[i + 8] = b.2.to_be_bytes().as_slice()[i];
                a_bytes[i + 16] = a.1.to_be_bytes().as_slice()[i];
                b_bytes[i + 16] = b.1.to_be_bytes().as_slice()[i];
                a_bytes[i + 24] = a.0.to_be_bytes().as_slice()[i];
                b_bytes[i + 24] = b.0.to_be_bytes().as_slice()[i];
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
            let res_len = res_be.len();
            let res_ptr: *mut u8 = res_be.as_mut_ptr();
            let res_vec_ptr = res_vec.as_ptr();
            for i in 0..res_vec_idx {
                unsafe {
                    *res_ptr.offset((res_len - res_vec_idx + i) as isize) =
                        *res_vec_ptr.offset(i as isize);
                }
            }

            if a_sign != b_sign {
                let mut carry = true;
                for i in (0..res_be.len()).rev() {
                    res_be[i] = !res_be[i];
                    if carry {
                        if res_be[i] == BYTE_MAX_VAL as u8 {
                            res_be[i] = 0;
                        } else {
                            res_be[i] += 1;
                            carry = false;
                        };
                    }
                }
            }
            let mut v = [0u8; 8];
            v.clone_from_slice(&res_be[24..32]);
            result_le.0 = u64::from_be_bytes(v);
            v.clone_from_slice(&res_be[16..24]);
            result_le.1 = u64::from_be_bytes(v);
            v.clone_from_slice(&res_be[8..16]);
            result_le.2 = u64::from_be_bytes(v);
            v.clone_from_slice(&res_be[0..8]);
            result_le.3 = u64::from_be_bytes(v);
        }
    }

    result_le
}
