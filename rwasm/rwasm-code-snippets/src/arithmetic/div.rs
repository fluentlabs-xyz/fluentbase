#[no_mangle]
fn arithmetic_div(
    mut a0: u64,
    mut a1: u64,
    mut a2: u64,
    mut a3: u64,
    mut b0: u64,
    mut b1: u64,
    mut b2: u64,
    mut b3: u64,
) -> (u64, u64, u64, u64) {
    fn bits_u64_limbs(x: (u64, u64, u64, u64)) -> usize {
        let limbs = [x.0, x.1, x.2, x.3];
        let mut bits = 0;

        for &limb in limbs.iter().rev() {
            bits += 64 - limb.leading_zeros() as usize;
        }

        bits
    }

    let mut result = [0u64; 4];

    let mut my_bits = bits_u64_limbs((a0, a1, a2, a3));
    let your_bits = bits_u64_limbs((b0, b1, b2, b3));

    assert_eq!(my_bits, 100);
    //my_bits = 100;
    assert_eq!(your_bits, 92);
    // check for division by 0
    assert!(your_bits != 0);

    if my_bits < your_bits {
        return result.into();
    }

    // Bitwise long division
    let mut shift = my_bits - your_bits;

    b0 <<= shift;
    b1 <<= shift;
    b2 <<= shift;
    b3 <<= shift;
    loop {
        if compare_limbs(a0, a1, a2, a3, b0, b1, b2, b3) >= 0 {
            result[shift / 64] |= 1 << (shift % 64);

            a0 = a0.wrapping_sub(b0);
            a1 = a1.wrapping_sub(b1);
            a2 = a2.wrapping_sub(b2);
            a3 = a3.wrapping_sub(b3);
        }

        b0 >>= 1;
        b1 >>= 1;
        b2 >>= 1;
        b3 >>= 1;

        if shift == 0 {
            break;
        }
        shift -= 1;
    }

    result.into()
}

fn compare_limbs(a0: u64, a1: u64, a2: u64, a3: u64, b0: u64, b1: u64, b2: u64, b3: u64) -> i32 {
    if a3 < b3 {
        return -1;
    } else if a3 > b3 {
        return 1;
    }
    if a2 < b2 {
        return -1;
    } else if a2 > b2 {
        return 1;
    }
    if a1 < b1 {
        return -1;
    } else if a1 > b1 {
        return 1;
    }
    if a0 < b0 {
        return -1;
    } else if a0 > b0 {
        return 1;
    }
    0
}

#[test]
fn test_xx() {
    use ethereum_types::U256;

    let u256_x = U256::from_dec_str("1200000000000000000000000000000").unwrap();
    let u256_y = U256::from_dec_str("3000000000000000000000000000").unwrap();

    println!("{:?}", u256_x.bits());
    println!("{:?}", u256_y.bits());

    // split the U256 into 4 u64 values
    let (u64_x_0, u64_x_1, u64_x_2, u64_x_3) = split_u256_be(u256_x);
    let (u64_y_0, u64_y_1, u64_y_2, u64_y_3) = split_u256_be(u256_y);

    println!("{:?}", (u64_x_0, u64_x_1, u64_x_2, u64_x_3));

    let (res_0, res_1, res_2, res_3) = arithmetic_div(
        u64_x_0, u64_x_1, u64_x_2, u64_x_3, u64_y_0, u64_y_1, u64_y_2, u64_y_3,
    );
    println!("{:?}", (res_0, res_1, res_2, res_3));

    println!("RES: {:?}", combine_u64(res_0, res_1, res_2, res_3));
}

// // #[derive(Debug, Copy, Clone)]
// // struct U256 {
// //     parts: [u64; 4],
// // }

// fn split_u256(u256: U256) -> (u64, u64, u64, u64) {
//     let limb0 = u256.low_u64();
//     let limb1 = u256.0[1];
//     let limb2 = u256.0[2];
//     let limb3 = u256.0[3];

//     (limb0, limb1, limb2, limb3)
// }

// fn combine_u64(u64_0: u64, u64_1: u64, u64_2: u64, u64_3: u64) -> U256 {
//     // Create a new U256 using the provided u64 values
//     U256([u64_0, u64_1, u64_2, u64_3])
// }

// fn div(first, other: U256) -> U256 {
//     let mut sub_copy = first;
//     let mut shift_copy = other;
//     let mut ret = [0u64; 4];

//     let my_bits = first.bits();
//     let your_bits = other.bits();

//     // Check for division by 0
//     assert!(your_bits != 0);

//     // Early return in case we are dividing by a larger number than us
//     if my_bits < your_bits {
//         return U256(ret);
//     }

//     // Bitwise long division
//     let mut shift = my_bits - your_bits;
//     shift_copy = shift_copy << shift;
//     loop {
//         if sub_copy >= shift_copy {
//             ret[shift / 64] |= 1 << (shift % 64);
//             sub_copy = sub_copy - shift_copy;
//         }
//         shift_copy = shift_copy >> 1;
//         if shift == 0 { break; }
//         shift -= 1;
//     }

//     U256(ret)
// }

// fn div(
//     x1: u64,
//     x2: u64,
//     x3: u64,
//     x4: u64,
//     y1: u64,
//     y2: u64,
//     y3: u64,
//     y4: u64,
// ) -> (u64, u64, u64, u64) {
//     let mut sub_copy = [x1, x2, x3, x4];
//     let mut shift_copy = [y1, y2, y3, y4];
//     let mut ret = (0u64, 0u64, 0u64, 0u64);

//     let my_bits = sub_copy
//         .iter()
//         .rev()
//         .cloned()
//         .fold(0, |acc, x| acc * 64 + x.leading_zeros() as usize);
//     let your_bits = shift_copy
//         .iter()
//         .rev()
//         .cloned()
//         .fold(0, |acc, x| acc * 64 + x.leading_zeros() as usize);

//     // Check for division by 0
//     assert!(your_bits != 0);

//     // Early return in case we are dividing by a larger number than us
//     if my_bits < your_bits {
//         return (0, 0, 0, 0);
//     }

//     // Bitwise long division
//     let mut shift = (u256_x.bits() as i32 - u256_y.bits() as i32) as usize;
//     let mut shift_copy = u256_y << shift;

//     for _ in 0..my_bits {
//         let compare_result = compare(&sub_copy, &shift_copy);
//         if compare_result == 1 || compare_result == 0 {
//             ret = (
//                 ret.0 | (1 << (shift % 64)),
//                 ret.1 | (1 << (shift % 64)),
//                 ret.2 | (1 << (shift % 64)),
//                 ret.3 | (1 << (shift % 64)),
//             );
//             sub_copy = subtract(&sub_copy, &shift_copy);
//         }

//         for i in (0..4).rev() {
//             shift_copy[i] >>= 1;
//         }

//         shift -= 1;
//     }

//     ret
// }

// // Compare two arrays of u64
// fn compare(x: &[u64; 4], y: &[u64; 4]) -> i32 {
//     for i in (0..4).rev() {
//         if x[i] > y[i] {
//             return 1;
//         } else if x[i] < y[i] {
//             return -1;
//         }
//     }
//     0
// }

// // Subtract two arrays of u64, assuming x >= y
// fn subtract(x: &[u64; 4], y: &[u64; 4]) -> [u64; 4] {
//     let mut result = [0u64; 4];
//     let mut borrow = 0i64;

//     for i in 0..4 {
//         let (sub, new_borrow) = x[i].overflowing_sub(y[i] + borrow as u64);
//         result[i] = sub;
//         borrow = new_borrow as i64;
//     }

//     result
// }

// #[test]
// fn test_arithmetic_div() {
//     // Example usage
//     let u256_x = U256::from_dec_str("6000000000000000000000000000").unwrap();
//     let u256_y = U256::from_dec_str("2000000000000000000000000000").unwrap();

//     // Split the U256 into 4 u64 values
//     let (u64_x_0, u64_x_1, u64_x_2, u64_x_3) = split_u256(u256_x);
//     let (u64_y_0, u64_y_1, u64_y_2, u64_y_3) = split_u256(u256_y);

//     let (result_0, result_1, result_2, result_3) = div(
//         u64_x_0, u64_x_1, u64_x_2, u64_x_3, u64_y_0, u64_y_1, u64_y_2, u64_y_3,
//     );

//     println!("u64_0: {:x}", result_0);
//     println!("u64_1: {:x}", result_1);
//     println!("u64_2: {:x}", result_2);
//     println!("u64_3: {:x}", result_3);

//     // Use the result as needed
//     println!(
//         "Result {:?}",
//         combine_u64(result_0, result_1, result_2, result_3)
//     );
// }

// #[no_mangle]
// fn arithmetic_mul(x1: u64, x2: u64, x3: u64, x4: u64, y1: u64, y2: u64, y3: u64, y4: u64) -> U256
// {     let mut result = U256::zero();

//     // for i in 0..3 {
//     let mut carry = 0u64;

//     fn update_and_get_overflow(result: &mut (u64, u64, u64, u64), index: u64, low: u64) -> bool {
//         let existing_low = &mut result.0[index];
//         let (new_low, overflow) = low.overflowing_add(*existing_low);
//         *existing_low = new_low;
//         overflow
//     }

//     // b = {y1, y2, y3 ,y4}

//     // let b = y.0[i];

//     // //  for j in 0..3 {
//     // // if i + j < 4 {
//     // // Ensure not to go out of bounds
//     // let a = x.0[j];

//     let (hi, low) = split_u128(x1 as u128 * y1 as u128);

//     //

//     let (hi, low) = split_u128(x2 as u128 * y2 as u128);

// //
//     let (hi, low) = split_u128(x3 as u128 * y3 as u128);
//     let (hi, low) = split_u128(x4 as u128 * y4 as u128);

//     let (hi, low) = split_u128(x2 as u128 * y2 as u128);
//     let (hi, low) = split_u128(x3 as u128 * y3 as u128);
//     let (hi, low) = split_u128(x4 as u128 * y4 as u128);

//     let (hi, low) = split_u128(x3 as u128 * y3 as u128);
//     let (hi, low) = split_u128(x4 as u128 * y4 as u128);

//     let (hi, low) = split_u128(x3 as u128 * y3 as u128);

//     let overflow = {
//         let existing_low = &mut result.0[i + j];
//         let (low, o) = low.overflowing_add(*existing_low);
//         *existing_low = low;
//         o
//     };

//     carry = {
//         // if i + j < 3 {
//         let existing_hi = &mut result.0[i + j + 1];
//         let hi = hi + overflow as u64;
//         let (hi, o0) = hi.overflowing_add(carry);
//         let (hi, o1) = hi.overflowing_add(*existing_hi);
//         *existing_hi = hi;

//         (o0 | o1) as u64;
//         // } else {
//         overflow as u64
//         //}
//     };
//     //  }
//     //    }
//     //  }

//     result
// }
