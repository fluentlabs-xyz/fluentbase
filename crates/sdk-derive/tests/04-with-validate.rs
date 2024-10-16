use fluentbase_sdk_derive::function_id;

#[function_id("transfer(address,uint256)", validate(true))]
fn transfer_validate() {}

fn main() {
    assert_eq!(FUNCTION_ID_HEX, "0xa9059cbb");
    assert_eq!(FUNCTION_ID_BYTES, [169, 5, 156, 187]);
    assert_eq!(FUNCTION_SIGNATURE, "transfer(address,uint256)");
}
