use trybuild::TestCases;

#[test]
fn function_id_tests() {
    let t = TestCases::new();
    t.pass("tests/router/solidity/01-basic-usage.rs");
}
