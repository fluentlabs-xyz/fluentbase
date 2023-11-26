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
    let mut res = 0;
    if a0 > b0 {
        res = 1;
    } else if a0 < b0 {
        res = 0;
    } else if a1 > b1 {
        res = 1;
    } else if a1 < b1 {
        res = 0;
    } else if a2 > b2 {
        res = 1;
    } else if a2 < b2 {
        res = 0;
    } else if a3 > b3 {
        res = 1;
    }

    return (0, 0, 0, res);
}
