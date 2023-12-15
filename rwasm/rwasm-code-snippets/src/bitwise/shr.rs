use crate::common::shr;

#[no_mangle]
fn bitwise_shr(
    v0: u64,
    v1: u64,
    v2: u64,
    v3: u64,
    shift0: u64,
    shift1: u64,
    shift2: u64,
    shift3: u64,
) -> (u64, u64, u64, u64) {
    shr(shift0, shift1, shift2, shift3, v0, v1, v2, v3)
}
