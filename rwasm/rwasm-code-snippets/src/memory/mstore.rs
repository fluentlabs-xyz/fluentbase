#[no_mangle]
fn memory_mstore(
    offset0: u64,
    offset1: u64,
    offset2: u64,
    offset3: u64,
    b0: u64,
    b1: u64,
    b2: u64,
    b3: u64,
) -> (u64, u64, u64, u64) {
    let s0;
    if offset0 == b0 && offset1 == b1 && offset2 == b2 && offset3 == b3 {
        s0 = 1;
    } else {
        s0 = 0;
    }

    return (s0, 0, 0, 0);
}
