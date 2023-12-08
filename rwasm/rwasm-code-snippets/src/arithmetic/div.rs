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
    fn subtract_with_remainder(a: &mut [u8], b: &[u8]) -> bool {
        if b.len() > a.len() {
            return false;
        }
        // check if can subtract
        for ai in 0..a.len() {
            let bi = (b.len() as i32 - a.len() as i32) + ai as i32;
            if bi < 0 {
                if a[ai] > 0 {
                    break;
                }
            } else {
                if a[ai] > b[bi as usize] {
                    break;
                } else if a[ai] < b[bi as usize] {
                    return false;
                }
            }
        }
        let mut borrow = 0;
        let mut ai: i32 = a.len() as i32 - 1;
        let mut bi: i32 = b.len() as i32 - 1;
        while ai >= 0 {
            if bi < 0 {
                if borrow <= 0 {
                    return true;
                }
                if a[ai as usize] >= borrow {
                    a[ai as usize] -= borrow;
                    borrow = 0
                } else {
                    a[ai as usize] = BYTE_MAX_VAL as u8
                }
                ai -= 1;
                continue;
            }
            if a[ai as usize] >= b[bi as usize] + borrow {
                a[ai as usize] -= b[bi as usize] + borrow;
                borrow = 0
            } else {
                a[ai as usize] =
                    BYTE_MAX_VAL as u8 - b[bi as usize] + a[ai as usize] + (1 - borrow);
                borrow = 1;
            }
            ai -= 1;
            bi -= 1;
        }
        if borrow > 0 {
            panic!("borrow is still greater 0"); //impossible
        }
        true
    }

    #[inline]
    fn try_divide(dividend_remainder: &mut [u8], divisor: &[u8]) -> u8 {
        let mut res: u8 = 0;
        while subtract_with_remainder(dividend_remainder, divisor) {
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
        let mut res_vec = [0u8; 32];
        let mut res_vec_idx: usize = 0;
        let mut a_bytes: &mut [u8] = &mut [0u8; U256_BYTES_COUNT as usize];
        let mut b_bytes: &mut [u8] = &mut [0u8; U256_BYTES_COUNT as usize];

        a_bytes[0..8].clone_from_slice(a3.to_be_bytes().as_slice());
        a_bytes[8..16].clone_from_slice(a2.to_be_bytes().as_slice());
        a_bytes[16..24].clone_from_slice(a1.to_be_bytes().as_slice());
        a_bytes[24..32].clone_from_slice(a0.to_be_bytes().as_slice());

        b_bytes[0..8].clone_from_slice(b3.to_be_bytes().as_slice());
        b_bytes[8..16].clone_from_slice(b2.to_be_bytes().as_slice());
        b_bytes[16..24].clone_from_slice(b1.to_be_bytes().as_slice());
        b_bytes[24..32].clone_from_slice(b0.to_be_bytes().as_slice());

        let mut sub_pos = 0;
        for i in 0..a_bytes.len() {
            if a_bytes[i] != 0 {
                sub_pos = i as u64;
                break;
            }
        }
        if sub_pos > 0 {
            a_bytes = &mut a_bytes[sub_pos as usize..];
        }

        sub_pos = 0;
        for i in 0..U256_BYTES_COUNT as usize {
            if b_bytes[i] != 0 {
                sub_pos = i as u64;
                break;
            }
        }
        if sub_pos > 0 {
            b_bytes = &mut b_bytes[sub_pos as usize..];
        }
        let mut a_pos_start = 0;
        let mut a_pos_end = a_pos_start + b_bytes.len();

        loop {
            let mut a_chunk = &mut a_bytes[a_pos_start..a_pos_end];
            // debug!(
            //     "a_pos_start={} a_pos_end={} a_chunk({})={:x?} b_bytes({})={:x?}",
            //     a_pos_start,
            //     a_pos_end,
            //     a_chunk.len(),
            //     a_chunk,
            //     b_bytes.len(),
            //     b_bytes,
            // );
            let div_res = try_divide(&mut a_chunk, b_bytes);
            // debug!("a_chunk/b_bytes({}) = {:x?}", a_chunk.len(), a_chunk);
            // debug!("div_res={:?}\n\n", div_res);
            res_vec[res_vec_idx] = div_res;
            res_vec_idx += 1;

            a_pos_end += 1;
            if div_res > 0 {
                for v in &a_bytes[a_pos_start..] {
                    if *v != 0 {
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
        res[res_len - res_vec_idx..].clone_from_slice(&res_vec[0..res_vec_idx]);
        // println!("res {:?} \n\n", res);
        let mut v = [0u8; 8];
        for i in 0..4 {
            v.clone_from_slice(&res[24 - i * 8..32 - i * 8]);
            result[i] = u64::from_be_bytes(v);
        }
    }

    (result[0], result[1], result[2], result[3])
}
