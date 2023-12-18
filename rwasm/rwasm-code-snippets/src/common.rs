use crate::{
    consts::{
        BYTE_MAX_VAL,
        U256_BYTES_COUNT,
        U64_ALL_BITS_ARE_1,
        U64_HALF_BITS_COUNT,
        U64_LOW_PART_MASK,
        U64_MSBIT_IS_1,
    },
    global_var,
};

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

pub(crate) fn div_le(a: (u64, u64, u64, u64), b: (u64, u64, u64, u64)) -> (u64, u64, u64, u64) {
    let mut result = [0u64, 0u64, 0u64, 0u64];

    if b.3 == 0 && b.2 == 0 && b.1 == 0 && (b.0 == 1 || b.0 == 0) {
        if b.0 != 0 {
            result[0] = a.0;
            result[1] = a.1;
            result[2] = a.2;
            result[3] = a.3;
        }
    } else if a.3 == b.3 && a.2 == b.2 && a.1 == b.1 && a.0 == b.0 {
        if a.0 != 0 {
            result[0] = 1
        }
    } else if a.3 > b.3
        || (a.3 == b.3 && a.2 > b.2)
        || (a.3 == b.3 && a.2 == b.2 && a.1 > b.1)
        || (a.3 == b.3 && a.2 == b.2 && a.1 == b.1 && a.0 > b.0)
    {
        let mut res = &mut [0u8; U256_BYTES_COUNT as usize];
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
        let a_bytes_ptr = a_bytes.as_mut_ptr();
        let b_bytes_ptr = b_bytes.as_mut_ptr();
        loop {
            let a_len = a_pos_end - a_pos_start;
            let b_len = b_bytes.len() - b_pos_start;
            let div_res = try_divide_close_numbers(
                unsafe { a_bytes_ptr.offset(a_pos_start as isize) },
                a_len,
                unsafe { b_bytes_ptr.offset(b_pos_start as isize) },
                b_len,
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
        let mut v = [0u8; 8];
        for i in 0..4 {
            v.clone_from_slice(&res[24 - i * 8..32 - i * 8]);
            result[i] = u64::from_be_bytes(v);
        }
    }

    (result[0], result[1], result[2], result[3])
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
    unsafe { global_var = 654321 };
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
    /*if a_len < U128_BYTES_COUNT && b_len < U128_BYTES_COUNT {
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

pub(crate) fn add_global_mem(
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

// #[no_mangle]
pub(crate) fn exp(
    v0: u64,
    v1: u64,
    v2: u64,
    v3: u64,
    exp0: u64,
    exp1: u64,
    exp2: u64,
    exp3: u64,
) -> (u64, u64, u64, u64) {
    let mut r: (u64, u64, u64, u64) = (0, 0, 0, 0);

    if v3 == 0 && v2 == 0 && v1 == 0 && (v0 == 0 || v0 == 1) {
        if v0 == 0 {
            if exp3 == 0 && exp2 == 0 && exp1 == 0 && exp0 == 0 {
                r.0 = 1;
            }
        } else {
            r.0 = 1
        }
    } else if exp3 == 0 && exp2 == 0 && exp1 == 0 && (exp0 == 0 || exp0 == 1) {
        if exp0 == 1 {
            r = (v0, v1, v2, v3);
        } else {
            r.0 = 1
        }
    } else {
        let mut base: (u64, u64, u64, u64) = (v0, v1, v2, v3);
        let mut rp: (u64, u64, u64, u64) = (1, 0, 0, 0);
        let mut exp: (u64, u64, u64, u64) = (exp0, exp1, exp2, exp3);
        r.0 = 1;
        let mut c = 0;
        loop {
            c += 1;
            // TODO wrong condition, fix it
            if (exp.0 & 1) > 0 {
                // rX=rX*baseX
                r = mul(r.0, r.1, r.2, r.3, base.0, base.1, base.2, base.3);

                if r == rp {
                    break;
                }
                rp = r;
            }
            // expX >>=1
            exp.0 = (exp.0 >> 1) | (exp.1 << 63);
            exp.1 = (exp.1 >> 1) | (exp.2 << 63);
            exp.2 = (exp.2 >> 1) | (exp.3 << 63);
            exp.3 = exp.3 >> 1;
            if exp == (0, 0, 0, 0) {
                break;
            }
            // baseX=baseX*baseX
            base = mul(
                base.0, base.1, base.2, base.3, base.0, base.1, base.2, base.3,
            );
        }
    }

    r
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
            let a_len = a_pos_end - a_pos_start;
            let b_len = b_bytes.len() - b_pos_start;
            let div_res = try_divide_close_numbers(
                unsafe { a_bytes_ptr.offset(a_pos_start as isize) },
                a_len,
                unsafe { b_bytes_ptr.offset(b_pos_start as isize) },
                b_len,
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
        let mut v = [0u8; 8];
        for i in 0..4 {
            v.clone_from_slice(&a_bytes[i * 8..(i + 1) * 8]);
            result[3 - i] = u64::from_be_bytes(v);
        }
    }

    (result[0], result[1], result[2], result[3])
}

pub(crate) fn smod(
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
            (a3, a2, a1, a0) = convert_sign_be((a3, a2, a1, a0));
        }
        if b_sign {
            (b3, b2, b1, b0) = convert_sign_be((b3, b2, b1, b0));
        }
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
            let a_len = a_pos_end - a_pos_start;
            let b_len = b_bytes.len() - b_pos_start;
            let div_res = try_divide_close_numbers(
                unsafe { a_bytes_ptr.offset(a_pos_start as isize) },
                a_len,
                unsafe { b_bytes_ptr.offset(b_pos_start as isize) },
                b_len,
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

pub(crate) fn mul(
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

pub(crate) fn shr(
    shift0: u64,
    shift1: u64,
    shift2: u64,
    shift3: u64,
    v0: u64,
    v1: u64,
    v2: u64,
    v3: u64,
) -> (u64, u64, u64, u64) {
    let mut s0: u64 = 0;
    let mut s1: u64 = 0;
    let mut s2: u64 = 0;
    let mut s3: u64 = 0;

    if shift3 != 0 || shift2 != 0 || shift1 != 0 || shift0 > BYTE_MAX_VAL {
        // return (0, 0, 0, 0);
    } else if shift0 >= 192 {
        let shift = shift0 - 192;
        s0 = v3 >> shift;
        // return (0, 0, 0, s3);
    } else if shift0 >= 128 {
        let shift = shift0 - 128;
        let shift_inv = 64 - shift;
        s1 = v3 >> shift;
        s0 = v3 << shift_inv | v2 >> shift;
        // return (0, 0, s2, s3);
    } else if shift0 >= 64 {
        let shift = shift0 - 64;
        let shift_inv = 64 - shift;
        s2 = v3 >> shift;
        s1 = v3 << shift_inv | v2 >> shift;
        s0 = v2 << shift_inv | v1 >> shift;
        // return (0, s1, s2, s3);
    } else {
        let shift = shift0;
        let shift_inv = 64 - shift;
        s3 = v3 >> shift;
        s2 = v3 << shift_inv | v2 >> shift;
        s1 = v2 << shift_inv | v1 >> shift;
        s0 = v1 << shift_inv | v0 >> shift;
    }

    (s0, s1, s2, s3)
}

pub(crate) fn convert_sign_be(v: (u64, u64, u64, u64)) -> (u64, u64, u64, u64) {
    let mut r = v;
    let sign = v.0 & U64_MSBIT_IS_1 > 0;
    if sign {
        if r.3 < 1 {
            r.3 = U64_ALL_BITS_ARE_1;

            if r.2 < 1 {
                r.2 = U64_ALL_BITS_ARE_1;
                if r.1 < 1 {
                    r.1 = U64_ALL_BITS_ARE_1;
                    if r.0 < 1 {
                        r.0 = U64_ALL_BITS_ARE_1;
                    } else {
                        r.0 -= 1;
                    }
                } else {
                    r.1 -= 1;
                }
            } else {
                r.2 -= 1;
            }
        } else {
            r.3 -= 1;
        }
        r.0 = !r.0;
        r.1 = !r.1;
        r.2 = !r.2;
        r.3 = !r.3;
    } else {
        r.0 = !r.0;
        r.1 = !r.1;
        r.2 = !r.2;
        r.3 = !r.3;
        if r.3 == U64_ALL_BITS_ARE_1 {
            r.3 = 0;
            if r.2 == U64_ALL_BITS_ARE_1 {
                r.2 = 0;
                if r.1 == U64_ALL_BITS_ARE_1 {
                    r.1 = 0;
                    r.0 += 1;
                } else {
                    r.1 += 1;
                }
            } else {
                r.2 += 1;
            }
        } else {
            r.3 += 1;
        }
    }
    r
}

pub(crate) fn convert_sign_le(v: (u64, u64, u64, u64)) -> (u64, u64, u64, u64) {
    let mut r = v;
    let sign = v.3 & U64_MSBIT_IS_1 > 0;
    if sign {
        if r.0 < 1 {
            r.0 = U64_ALL_BITS_ARE_1;

            if r.1 < 1 {
                r.1 = U64_ALL_BITS_ARE_1;
                if r.2 < 1 {
                    r.2 = U64_ALL_BITS_ARE_1;
                    if r.3 < 1 {
                        r.3 = U64_ALL_BITS_ARE_1;
                    } else {
                        r.3 -= 1;
                    }
                } else {
                    r.2 -= 1;
                }
            } else {
                r.1 -= 1;
            }
        } else {
            r.0 -= 1;
        }
        r.3 = !r.3;
        r.2 = !r.2;
        r.1 = !r.1;
        r.0 = !r.0;
    } else {
        r.3 = !r.3;
        r.2 = !r.2;
        r.1 = !r.1;
        r.0 = !r.0;
        if r.0 == U64_ALL_BITS_ARE_1 {
            r.0 = 0;
            if r.1 == U64_ALL_BITS_ARE_1 {
                r.1 = 0;
                if r.2 == U64_ALL_BITS_ARE_1 {
                    r.2 = 0;
                    r.3 += 1;
                } else {
                    r.2 += 1;
                }
            } else {
                r.1 += 1;
            }
        } else {
            r.0 += 1;
        }
    }
    r
}
