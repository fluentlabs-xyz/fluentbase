use fluentbase_sdk_derive::function_id;

#[function_id("0xa9059cbb")]
fn transfer_hex() {}

fn main() {
    assert_eq!(FUNCTION_ID_HEX, "0xa9059cbb");
    assert_eq!(FUNCTION_ID_BYTES, [169, 5, 156, 187]);
}
