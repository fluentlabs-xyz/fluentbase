use fluentbase_sdk_derive::function_id;

#[function_id([169, 5, 156, 187])]
fn transfer_bytes() {}

fn main() {
    assert_eq!(FUNCTION_ID_HEX, "0xa9059cbb");
    assert_eq!(FUNCTION_ID_BYTES, [169, 5, 156, 187]);
}
