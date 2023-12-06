use crate::consts::BYTE_MAX_VAL;

#[no_mangle]
fn memory_mstore8(
    value0: u64,
    value1: u64,
    value2: u64,
    value3: u64,
    offset0: u64,
    offset1: u64,
    offset2: u64,
    offset3: u64,
) {
    let v = (value0 & BYTE_MAX_VAL) as u8;
    let ptr = offset0 as *mut u8;
    unsafe { *ptr = v };
}
