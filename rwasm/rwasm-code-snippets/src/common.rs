use std::slice;

pub const STACK_POINTER_DEFAULT_MEM_OFFSET: usize = 0;

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
