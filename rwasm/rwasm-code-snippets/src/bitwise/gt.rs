#[no_mangle]
fn bitwise_gt(
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
    b0: u64,
    b1: u64,
    b2: u64,
    b3: u64,
) -> (u64, u64, u64, u64) {
    let mut s0 = 0;
    if a3 > b3 {
        s0 = 1;
    } else if a3 < b3 {
        s0 = 0;
    } else if a2 > b2 {
        s0 = 1;
    } else if a2 < b2 {
        s0 = 0;
    } else if a1 > b1 {
        s0 = 1;
    } else if a1 < b1 {
        s0 = 0;
    } else if a0 > b0 {
        s0 = 1;
    }

    (s0, 0, 0, 0)
}
