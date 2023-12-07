#[no_mangle]
pub fn arithmetic_div(
    b0: u64,
    b1: u64,
    b2: u64,
    b3: u64,
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
) -> (u64, u64, u64, u64) {
    let mut result = [0u64, 0u64, 0u64, 0u64];

    if a0 == b0 && a1 == b1 && a2 == b2 && a3 == b3 {
        if a0 != 0 {
            result[0] = 1
        }
    } else if a0 < b0 {
    } else if a1 < b1 {
    } else if a2 < b2 {
    } else if a3 < b3 {
    } else {
        // need specific processing
    }

    (result[0], result[1], result[2], result[3])
}
