use fluentbase_sdk_derive::function_id;

#[function_id([1, 2, 3])] // Should be 4 bytes
fn invalid_bytes() {}

fn main() {}
