use trybuild::TestCases;

#[test]
fn test_router_solidity() {
    let t = TestCases::new();
    t.pass("tests/router/solidity/01-basic-usage.rs");
}
