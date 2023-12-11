// use std::slice;

use crate::consts::{
    BYTE_MAX_VAL,
    U256_BYTES_COUNT,
    U64_HALF_BITS_COUNT,
    U64_LOW_PART_MASK,
    U64_MSBIT_IS_1,
};

// #[no_mangle]
// #[inline]
// pub fn stack_pointer_value_get(mem_offset: usize) -> *mut i32 {
//     let mut mem: &mut [i32];
//     unsafe {
//         mem = slice::from_raw_parts_mut(mem_offset as *mut i32, 1);
//     }
//     mem[0] as *mut i32
// }
//
// #[no_mangle]
// #[inline]
// pub fn stack_pointer_value_update(mem_offset: usize, value: i32) {
//     let mut mem: &mut [i32];
//     unsafe {
//         mem = slice::from_raw_parts_mut(mem_offset as *mut i32, 1);
//     }
//     mem[0] = value;
// }

#[inline]
pub(crate) fn subtract_with_remainder(
    a_start_ptr: *mut u8,
    a_len: usize,
    b_start_ptr: *mut u8,
    b_len: usize,
) -> bool {
    let mut res = false;
    if b_len <= a_len {
        // check if can subtract
        for ai in 0..a_len {
            let bi = (b_len as i32 - a_len as i32) + ai as i32;
            let a_val = unsafe { *a_start_ptr.offset(ai as isize) };
            if bi < 0 {
                if a_val > 0 {
                    break;
                }
            } else {
                let b_val = unsafe { *b_start_ptr.offset(bi as isize) };
                if a_val > b_val {
                    break;
                } else if a_val < b_val {
                    return false;
                }
            }
        }
        let mut borrow: u8 = 0;
        let mut ai = a_len as i32 - 1;
        let mut bi = b_len as i32 - 1;
        while ai >= 0 {
            if bi < 0 {
                if borrow <= 0 {
                    return true;
                }
                let a_val_ptr = unsafe { a_start_ptr.offset(ai as isize) };
                let a_val = unsafe { *a_val_ptr };
                if a_val >= borrow {
                    unsafe {
                        *a_val_ptr -= borrow;
                    }
                    borrow = 0
                } else {
                    unsafe { *a_val_ptr = BYTE_MAX_VAL as u8 }
                }
                ai -= 1;
            } else {
                let a_val_ptr = unsafe { a_start_ptr.offset(ai as isize) };
                let b_val = unsafe { *b_start_ptr.offset(bi as isize) };
                let a_val = unsafe { *a_val_ptr };
                // need cast because sum may be greater u8
                if a_val as u16 >= b_val as u16 + borrow as u16 {
                    unsafe {
                        *a_val_ptr -= b_val + borrow;
                    }
                    borrow = 0
                } else {
                    unsafe {
                        *a_val_ptr = BYTE_MAX_VAL as u8 - b_val + a_val + (1 - borrow);
                    }
                    borrow = 1;
                }
                ai -= 1;
                bi -= 1;
            }
        }
        res = true
    }
    // if borrow > 0 {
    //     panic!("borrow is still greater 0"); //impossible
    // }
    res
}

/// tries to divide two numbers which quotient must be less than u8::MAX.
/// saves result in a. doesnt panic in any problems - instead UB in such situations.
#[inline]
pub(crate) fn try_divide_close_numbers(
    a_start_ptr: *mut u8,
    a_len: usize,
    b_start_ptr: *mut u8,
    b_len: usize,
) -> u8 {
    let mut res: u8 = 0;
    const U128_BYTES_COUNT: usize = 16;
    const U64_BYTES_COUNT: usize = 8;
    if a_len < U64_BYTES_COUNT && b_len < U64_BYTES_COUNT {
        let mut a_bytes = [0u8; U64_BYTES_COUNT];
        let mut a_bytes_ptr = a_bytes.as_mut_ptr();
        unsafe {
            for i in 0..a_len {
                *a_bytes_ptr.offset((U64_BYTES_COUNT - a_len + i) as isize) =
                    *a_start_ptr.offset(i as isize);
            }
        }
        let mut b_bytes = [0u8; U64_BYTES_COUNT];
        let mut b_bytes_ptr = b_bytes.as_mut_ptr();
        for i in 0..b_len {
            unsafe {
                *b_bytes_ptr.offset((U64_BYTES_COUNT - b_len + i) as isize) =
                    *b_start_ptr.offset(i as isize);
            }
        }
        let mut a: u64 = 0;
        let mut b: u64 = 0;
        for i in 0..U64_BYTES_COUNT {
            a = a * 0x100 + unsafe { *a_bytes_ptr.offset(i as isize) as u64 };
            b = b * 0x100 + unsafe { *b_bytes_ptr.offset(i as isize) as u64 };
        }
        if b != 0 {
            res = (a / b) as u8;
            a = a - b * res as u64;
        }
        a_bytes = a.to_be_bytes();
        let mut a_bytes_ptr = a_bytes.as_ptr();
        for i in 0..a_len {
            unsafe {
                let v = *a_bytes_ptr.offset((U64_BYTES_COUNT - a_len + i) as isize);
                *a_start_ptr.offset(i as isize) = v;
            };
        }
    }
    /*else if a_len < U128_BYTES_COUNT && b_len < U128_BYTES_COUNT {
        let mut a_bytes = [0u8; U128_BYTES_COUNT];
        let mut a_bytes_ptr = a_bytes.as_mut_ptr();
        unsafe {
            for i in 0..a_len {
                *a_bytes_ptr.offset((U128_BYTES_COUNT - a_len + i) as isize) =
                    *a_start_ptr.offset(i as isize);
            }
        }
        let mut b_bytes = [0u8; U128_BYTES_COUNT];
        let mut b_bytes_ptr = b_bytes.as_mut_ptr();
        for i in 0..b_len {
            unsafe {
                *b_bytes_ptr.offset((U128_BYTES_COUNT - b_len + i) as isize) =
                    *b_start_ptr.offset(i as isize);
            }
        }
        let mut a: u128 = 0;
        let mut b: u128 = 0;
        for i in 0..U128_BYTES_COUNT {
            a = a * 0x100 + unsafe { *a_bytes_ptr.offset(i as isize) as u128 };
            b = b * 0x100 + unsafe { *b_bytes_ptr.offset(i as isize) as u128 };
        }
        if b != 0 {
            res = (a / b) as u8;
            a = a - b * res as u128;
        }
        a_bytes = a.to_be_bytes();
        let mut a_bytes_ptr = a_bytes.as_ptr();
        for i in 0..a_len {
            unsafe {
                let v = *a_bytes_ptr.offset((U128_BYTES_COUNT - a_len + i) as isize);
                *a_start_ptr.offset(i as isize) = v;
            };
        }
    }*/
    else {
        while subtract_with_remainder(a_start_ptr, a_len, b_start_ptr, b_len) {
            res += 1;
        }
    }
    res
}

pub(crate) fn add(
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
    b0: u64,
    b1: u64,
    b2: u64,
    b3: u64,
) -> (u64, u64, u64, u64) {
    let mut a_part: u64 = 0;
    let mut b_part: u64 = 0;
    let mut part_sum: u64 = 0;
    let mut carry: u64 = 0;
    let mut s0: u64 = 0;
    let mut s1: u64 = 0;
    let mut s2: u64 = 0;
    let mut s3: u64 = 0;

    a_part = a0 & U64_LOW_PART_MASK;
    b_part = b0 & U64_LOW_PART_MASK;
    part_sum = a_part + b_part;
    s0 = part_sum & U64_LOW_PART_MASK;
    carry = part_sum >> U64_HALF_BITS_COUNT;
    a_part = a0 >> U64_HALF_BITS_COUNT;
    b_part = b0 >> U64_HALF_BITS_COUNT;
    part_sum = a_part + b_part + carry;
    s0 = s0 + ((part_sum & U64_LOW_PART_MASK) << U64_HALF_BITS_COUNT);
    carry = part_sum >> U64_HALF_BITS_COUNT;

    a_part = a1 & U64_LOW_PART_MASK;
    b_part = b1 & U64_LOW_PART_MASK;
    part_sum = a_part + b_part + carry;
    s1 = part_sum & U64_LOW_PART_MASK;
    carry = part_sum >> U64_HALF_BITS_COUNT;
    a_part = a1 >> U64_HALF_BITS_COUNT;
    b_part = b1 >> U64_HALF_BITS_COUNT;
    part_sum = a_part + b_part + carry;
    s1 = s1 + ((part_sum & U64_LOW_PART_MASK) << U64_HALF_BITS_COUNT);
    carry = part_sum >> U64_HALF_BITS_COUNT;

    a_part = a2 & U64_LOW_PART_MASK;
    b_part = b2 & U64_LOW_PART_MASK;
    part_sum = a_part + b_part + carry;
    s2 = part_sum & U64_LOW_PART_MASK;
    carry = part_sum >> U64_HALF_BITS_COUNT;
    a_part = a2 >> U64_HALF_BITS_COUNT;
    b_part = b2 >> U64_HALF_BITS_COUNT;
    part_sum = a_part + b_part + carry;
    s2 = s2 + ((part_sum & U64_LOW_PART_MASK) << U64_HALF_BITS_COUNT);
    carry = part_sum >> U64_HALF_BITS_COUNT;

    a_part = a3 & U64_LOW_PART_MASK;
    b_part = b3 & U64_LOW_PART_MASK;
    part_sum = a_part + b_part + carry;
    s3 = part_sum & U64_LOW_PART_MASK;
    carry = part_sum >> U64_HALF_BITS_COUNT;
    a_part = a3 >> U64_HALF_BITS_COUNT;
    b_part = b3 >> U64_HALF_BITS_COUNT;
    part_sum = a_part + b_part + carry;
    s3 = s3 + ((part_sum & U64_LOW_PART_MASK) << U64_HALF_BITS_COUNT);

    (s0, s1, s2, s3)
}

#[inline]
pub(crate) fn mod_impl(
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
    b0: u64,
    b1: u64,
    b2: u64,
    b3: u64,
) -> (u64, u64, u64, u64) {
    let mut result = [0u64, 0u64, 0u64, 0u64];

    if a3 == b3 && a2 == b2 && a1 == b1 && a0 == b0 {
        result[0] = 0;
    } else if b3 == 0 && b2 == 0 && b1 == 0 && b0 == 1 {
        result[0] = 0;
    } else {
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
        let a_bytes_ptr = a_bytes.as_mut_ptr();
        let b_bytes_ptr = b_bytes.as_mut_ptr();
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
            let a_len = a_pos_end - a_pos_start;
            let b_len = b_bytes.len() - b_pos_start;
            let div_res = try_divide_close_numbers(
                unsafe { a_bytes_ptr.offset(a_pos_start as isize) },
                a_len,
                unsafe { b_bytes_ptr.offset(b_pos_start as isize) },
                b_len,
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
        // let res_len = res.len();
        // let res_ptr: *mut u8 = res.as_mut_ptr();
        // let res_vec_ptr = res_vec.as_ptr();
        // for i in 0..res_vec_idx {
        //     unsafe {
        //         *res_ptr.offset((res_len - res_vec_idx + i) as isize) =
        //             *res_vec_ptr.offset(i as isize);
        //     }
        // }
        // println!("res {:?} \n\n", res);
        let mut v = [0u8; 8];
        for i in 0..4 {
            v.clone_from_slice(&a_bytes[i * 8..(i + 1) * 8]);
            result[3 - i] = u64::from_be_bytes(v);
        }
    }

    (result[0], result[1], result[2], result[3])
}

pub fn smod(
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
    b0: u64,
    b1: u64,
    b2: u64,
    b3: u64,
) -> (u64, u64, u64, u64) {
    let a_sign = a3 & U64_MSBIT_IS_1 > 0;
    let b_sign = b3 & U64_MSBIT_IS_1 > 0;
    let mut result = [0u64, 0u64, 0u64, 0u64];

    if a3 == b3 && a2 == b2 && a1 == b1 && a0 == b0 || b3 == 0 && b2 == 0 && b1 == 0 && b0 == 1 {
        result[0] = 0;
    } else {
        let mut a0 = a0;
        let mut a1 = a1;
        let mut a2 = a2;
        let mut a3 = a3;
        let mut b0 = b0;
        let mut b1 = b1;
        let mut b2 = b2;
        let mut b3 = b3;
        if a_sign {
            let mut borrow = 0;
            if a0 <= 0 {
                a0 = U64_MSBIT_IS_1;
                borrow = 1;

                if a1 < borrow {
                    a1 = U64_MSBIT_IS_1;
                    if a2 < borrow {
                        a2 = U64_MSBIT_IS_1;
                        if a3 < borrow {
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
        // let mut res = &mut [0u8; U256_BYTES_COUNT as usize];
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
        let a_bytes_ptr = a_bytes.as_mut_ptr();
        let b_bytes_ptr = b_bytes.as_mut_ptr();
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
            let a_len = a_pos_end - a_pos_start;
            let b_len = b_bytes.len() - b_pos_start;
            let div_res = try_divide_close_numbers(
                unsafe { a_bytes_ptr.offset(a_pos_start as isize) },
                a_len,
                unsafe { b_bytes_ptr.offset(b_pos_start as isize) },
                b_len,
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
        // let res_len = res.len();
        // let res_ptr: *mut u8 = res.as_mut_ptr();
        // let res_vec_ptr = res_vec.as_ptr();
        // for i in 0..res_vec_idx {
        //     unsafe {
        //         *res_ptr.offset((res_len - res_vec_idx + i) as isize) =
        //             *res_vec_ptr.offset(i as isize);
        //     }
        // }
        // println!("res {:?} \n\n", res);
        if a_sign {
            let mut carry = true;
            for i in (0..a_bytes.len()).rev() {
                a_bytes[i] = !a_bytes[i];
                if carry {
                    if a_bytes[i] == BYTE_MAX_VAL as u8 {
                        a_bytes[i] = 0;
                    } else {
                        a_bytes[i] += 1;
                        carry = false;
                    };
                }
            }
        }
        let mut v = [0u8; 8];
        for i in 0..4 {
            v.clone_from_slice(&a_bytes[i * 8..(i + 1) * 8]);
            result[3 - i] = u64::from_be_bytes(v);
        }
    }

    (result[0], result[1], result[2], result[3])
}

pub fn mul(
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
    b0: u64,
    b1: u64,
    b2: u64,
    b3: u64,
) -> (u64, u64, u64, u64) {
    fn multiply_u64(a: u64, b: u64) -> (u64, u64) {
        let a_lo = a & U64_LOW_PART_MASK;
        let a_hi = a >> U64_HALF_BITS_COUNT;
        let b_lo = b & U64_LOW_PART_MASK;
        let b_hi = b >> U64_HALF_BITS_COUNT;

        let lo = a_lo * b_lo;
        let mid1 = a_lo * b_hi;
        let mid2 = a_hi * b_lo;
        let hi = a_hi * b_hi;

        let (mid_sum, hi_carry) = mid1.overflowing_add(mid2);
        let hi_carry = (hi_carry as u64) << 32;

        let lo_result = lo.overflowing_add(mid_sum << 32);
        let hi_result = hi + (mid_sum >> U64_HALF_BITS_COUNT) + hi_carry + lo_result.1 as u64;

        (hi_result, lo_result.0)
    }

    let mut res = [0u64; 4];
    let av = [a0, a1, a2, a3];
    let bv = [b0, b1, b2, b3];

    for i in 0..4 {
        let mut carry: u64 = 0;
        let b = bv[i];

        for j in 0..4 {
            let res_cur_idx = i + j;
            if res_cur_idx < 4 {
                let a = av[j];

                let (h, l) = multiply_u64(a, b);

                let res_chunk = &mut res[res_cur_idx];
                let (l, o) = l.overflowing_add(*res_chunk);
                carry += o as u64;
                *res_chunk = l;

                let res_next_idx = res_cur_idx + 1;
                if res_next_idx < 4 {
                    let res_chunk = &mut res[res_next_idx];
                    let (h, o) = h.overflowing_add(carry);
                    carry = o as u64;
                    let (h, o) = h.overflowing_add(*res_chunk);
                    carry += o as u64;
                    *res_chunk = h;
                }
            }
        }
    }

    (res[0], res[1], res[2], res[3])
}
