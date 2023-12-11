// use std::slice;

use crate::consts::BYTE_MAX_VAL;

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
