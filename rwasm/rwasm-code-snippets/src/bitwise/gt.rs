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
    if a0 > b0 {
        return (0, 0, 0, 1);
    }
    if a0 < b0 {
        return (0, 0, 0, 0);
    }
    if a1 > b1 {
        return (0, 0, 0, 1);
    }
    if a1 < b1 {
        return (0, 0, 0, 0);
    }
    if a2 > b2 {
        return (0, 0, 0, 1);
    }
    if a2 < b2 {
        return (0, 0, 0, 0);
    }
    if a3 > b3 {
        return (0, 0, 0, 1);
    }

    return (0, 0, 0, 0);
}
