#[no_mangle]
fn memory_mstore(
    value0: u64,
    value1: u64,
    value2: u64,
    value3: u64,
    offset0: u64,
    offset1: u64,
    offset2: u64,
    offset3: u64,
) -> (u64, u64, u64, u64) {
    (value0, value1, value2, value3)
}
