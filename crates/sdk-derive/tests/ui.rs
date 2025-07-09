#[ignore]
#[test]
fn test_ui() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/**/pass/*.rs");
    t.compile_fail("tests/ui/**/fail/*.rs");
}
