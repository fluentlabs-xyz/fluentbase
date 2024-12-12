#[allow(dead_code)]
pub(crate) fn print_hex_dump(data: &[u8]) {
    for (i, chunk) in data.chunks(32).enumerate() {
        println!("{:03x}: {}", i * 32, hex::encode(chunk));
    }
}

#[allow(dead_code)]
pub(crate) fn print_hex_dump_and_expected(actual: &[u8], expected: &[u8]) {
    println!("Actual:");
    print_hex_dump(actual);
    println!("\nExpected:");
    print_hex_dump(expected);
}
