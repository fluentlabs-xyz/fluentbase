use crate::consts::{BYTE_MAX_VAL, U256_BYTES_COUNT};

#[no_mangle]
pub fn arithmetic_div(
    b0: u64,
    b1: u64,
    b2: u64,
    b3: u64,
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
) -> (u64, u64, u64, u64) {
    #[inline]
    fn subtract_with_remainder(
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

    #[inline]
    fn try_divide(a_start_ptr: *mut u8, a_len: usize, b_start_ptr: *mut u8, b_len: usize) -> u8 {
        let mut res: u8 = 0;
        while subtract_with_remainder(a_start_ptr, a_len, b_start_ptr, b_len) {
            res += 1;
        }
        res
    }

    let mut result = [0u64, 0u64, 0u64, 0u64];

    if a0 == b0 && a1 == b1 && a2 == b2 && a3 == b3 {
        if a0 != 0 {
            result[0] = 1
        }
    } else if a3 > b3
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
            let div_res = try_divide(
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
        let mut v = [0u8; 8];
        for i in 0..4 {
            v.clone_from_slice(&res[24 - i * 8..32 - i * 8]);
            result[i] = u64::from_be_bytes(v);
        }
    }

    (result[0], result[1], result[2], result[3])
}
