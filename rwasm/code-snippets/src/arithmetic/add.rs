use crate::{
    common::{add, u256_be_to_tuple_le, u256_tuple_le_to_be},
    common_sp::{stack_pop_u256, stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
};

#[no_mangle]
pub fn arithmetic_add() {
    let a = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let b = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let a = u256_be_to_tuple_le(a);
    let b = u256_be_to_tuple_le(b);

    let r = add(a, b);

    let res = u256_tuple_le_to_be(r);

    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, res);
}

// #[cfg(test)]
// mod tests {
//     use crate::arithmetic::add::arithmetic_add_fast;
//
//     #[test]
//     fn test_add() {
//         let mut stack = [0u64; 12];
//         // a = 100,100,100,100
//         stack[0] = 100;
//         stack[1] = 100;
//         stack[2] = 100;
//         stack[3] = 100;
//         // b = 20,20,20,20
//         stack[4] = 20;
//         stack[5] = 20;
//         stack[6] = 20;
//         stack[7] = 20;
//         let sp = stack.as_ptr() as usize + (stack.len() - 1) * 8;
//         stack[stack.len() - 1] = (stack.len() * 8 - 8) as u64;
//         unsafe {
//             arithmetic_add_fast(sp);
//         }
//         assert_eq!(stack[4], 120);
//         assert_eq!(stack[5], 120);
//         assert_eq!(stack[6], 120);
//         assert_eq!(stack[7], 120);
//     }
// }
