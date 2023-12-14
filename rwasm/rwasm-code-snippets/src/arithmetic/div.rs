use crate::{common::try_divide_close_numbers, consts::U256_BYTES_COUNT};

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
    let mut result = [0u64, 0u64, 0u64, 0u64];

    if b3 == 0 && b2 == 0 && b1 == 0 && (b0 == 1 || b0 == 0) {
        if b0 != 0 {
            result[0] = a0;
            result[1] = a1;
            result[2] = a2;
            result[3] = a3;
        }
    } else if a3 == b3 && a2 == b2 && a1 == b1 && a0 == b0 {
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
